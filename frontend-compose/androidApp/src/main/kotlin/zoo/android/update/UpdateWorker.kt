package zoo.android.update

import android.content.Context
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.os.Build
import androidx.work.*
import io.ktor.client.*
import io.ktor.client.engine.okhttp.*
import io.ktor.client.plugins.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.utils.io.core.readText
import io.ktor.utils.io.readAvailable
import io.ktor.utils.io.readRemaining
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.withLock
import works.zli.zoo.BuildConfig

internal class UpdatePreferences(context: Context) {
    val values = context.getSharedPreferences("android-updates", Context.MODE_PRIVATE)
    var enabled: Boolean
        get() = values.getBoolean("enabled", false)
        set(value) {
            values.edit().putBoolean("enabled", value).apply()
        }

    var status: String
        get() = values.getString("status", "更新はまだ確認していません").orEmpty()
        set(value) {
            values.edit().putString("status", value).apply()
        }
}

internal object UpdateScheduler {
    private const val PERIODIC = "android-main-update"
    private const val MANUAL = "android-main-update-now"

    fun configure(context: Context, enabled: Boolean) {
        UpdatePreferences(context).enabled = enabled
        val manager = WorkManager.getInstance(context)
        if (enabled && !BuildConfig.DEBUG) {
            manager.enqueueUniquePeriodicWork(
                PERIODIC,
                ExistingPeriodicWorkPolicy.UPDATE,
                PeriodicWorkRequestBuilder<UpdateWorker>(6, TimeUnit.HOURS)
                    .setConstraints(
                        Constraints.Builder()
                            .setRequiredNetworkType(NetworkType.UNMETERED)
                            .setRequiresBatteryNotLow(true)
                            .build()
                    )
                    .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.MINUTES)
                    .build(),
            )
        } else {
            manager.cancelUniqueWork(PERIODIC)
            manager.cancelUniqueWork(MANUAL)
        }
    }

    fun checkNow(context: Context) {
        WorkManager.getInstance(context)
            .enqueueUniqueWork(
                MANUAL,
                ExistingWorkPolicy.KEEP,
                OneTimeWorkRequestBuilder<UpdateWorker>()
                    .setInputData(workDataOf("manual" to true))
                    .setConstraints(
                        Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()
                    )
                    .build(),
            )
    }
}

class UpdateWorker(context: Context, parameters: WorkerParameters) :
    CoroutineWorker(context, parameters) {
    override suspend fun doWork(): Result =
        mutex.withLock {
            val prefs = UpdatePreferences(applicationContext)
            val manual = inputData.getBoolean("manual", false)
            if (BuildConfig.DEBUG) {
                prefs.status = "自動更新は署名付き release 版で利用できます"
                return@withLock Result.failure()
            }
            if (!manual && !prefs.enabled) return@withLock Result.success()
            if (!shizukuReady()) {
                prefs.status = "Shizuku を起動し、Zoo に使用を許可してください"
                return@withLock Result.failure()
            }
            val directory = File(applicationContext.cacheDir, "updates").apply { mkdirs() }
            val apk = File(directory, "candidate.apk")
            val http =
                HttpClient(OkHttp) {
                    engine { config { followSslRedirects(false) } }
                    install(HttpTimeout) {
                        requestTimeoutMillis = 300_000
                        socketTimeoutMillis = 60_000
                    }
                }
            try {
                prefs.status = "main の更新を確認しています"
                val raw =
                    http.prepareGet(UPDATE_FEED).execute { response ->
                        check(response.status.value == 200) {
                            "更新情報を取得できません (HTTP ${response.status.value})"
                        }
                        response.bodyAsChannel().readRemaining(16_385).readText()
                    }
                require(raw.toByteArray().size <= 16_384) { "更新情報が大きすぎます" }
                val release = UpdateRelease.parse(raw)
                val current =
                    applicationContext.packageManager.installed(applicationContext.packageName)
                if (release.versionCode <= current.version()) {
                    prefs.status = "最新です (${BuildConfig.VERSION_NAME})"
                    return@withLock Result.success()
                }
                prefs.status = "${release.versionName} をダウンロードしています"
                val digest = MessageDigest.getInstance("SHA-256")
                var total = 0L
                http.prepareGet(release.url).execute { response ->
                    check(response.status.value == 200) {
                        "APK を取得できません (HTTP ${response.status.value})"
                    }
                    response.headers["Content-Length"]?.toLongOrNull()?.let {
                        require(it == release.size)
                    }
                    val channel = response.bodyAsChannel()
                    withContext(Dispatchers.IO) {
                        apk.outputStream().buffered().use { output ->
                            val buffer = ByteArray(64 * 1024)
                            while (true) {
                                currentCoroutineContext().ensureActive()
                                val count = channel.readAvailable(buffer)
                                if (count < 0) break
                                total += count
                                require(total <= release.size && total <= MAX_APK_BYTES) {
                                    "APK のサイズが一致しません"
                                }
                                digest.update(buffer, 0, count)
                                output.write(buffer, 0, count)
                            }
                        }
                    }
                }
                require(
                    total == release.size &&
                        digest.digest().joinToString("") { "%02x".format(it) } == release.sha256
                ) {
                    "APK のチェックサムが一致しません"
                }
                val archive =
                    applicationContext.packageManager.archive(apk) ?: error("APK を解析できません")
                requireMatchingApk(
                    applicationContext.packageName,
                    current.version(),
                    current.signers(),
                    archive.packageName,
                    archive.version(),
                    archive.signers(),
                    release,
                )
                currentCoroutineContext().ensureActive()
                if (!manual && !prefs.enabled) return@withLock Result.success()
                prefs.status = "${release.versionName} をインストールしています"
                val result = ShizukuInstaller(applicationContext).install(apk)
                check(result == "Success") { result }
                prefs.status = "${release.versionName} への更新が完了しました"
                Result.success()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                prefs.status = "更新できませんでした: ${failure.message.orEmpty().take(1000)}"
                if (!manual && runAttemptCount < 3) Result.retry() else Result.failure()
            } finally {
                http.close()
                apk.delete()
            }
        }

    companion object {
        private val mutex = kotlinx.coroutines.sync.Mutex()
    }
}

@Suppress("DEPRECATION")
private fun PackageManager.installed(name: String): PackageInfo =
    getPackageInfo(name, signingFlags())

@Suppress("DEPRECATION")
private fun PackageManager.archive(file: File): PackageInfo? =
    getPackageArchiveInfo(file.absolutePath, signingFlags())

@Suppress("DEPRECATION")
private fun signingFlags(): Int =
    if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES
    else PackageManager.GET_SIGNATURES

@Suppress("DEPRECATION")
private fun PackageInfo.version(): Long =
    if (Build.VERSION.SDK_INT >= 28) longVersionCode else versionCode.toLong()

@Suppress("DEPRECATION")
private fun PackageInfo.signers(): Set<String> =
    (if (Build.VERSION.SDK_INT >= 28) signingInfo?.apkContentsSigners else signatures)
        ?.map { sha256(it.toByteArray()) }
        ?.toSet()
        .orEmpty()
