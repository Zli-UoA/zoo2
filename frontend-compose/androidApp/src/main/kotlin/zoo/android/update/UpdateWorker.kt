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
import io.ktor.utils.io.readAvailable
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.withLock
import works.zli.zoo.BuildConfig
import zoo.android.appText

internal class UpdatePreferences(context: Context) {
    private val context = context.applicationContext
    val values = context.getSharedPreferences("android-updates", Context.MODE_PRIVATE)
    var enabled: Boolean
        get() = values.getBoolean("enabled", false)
        set(value) {
            values.edit().putBoolean("enabled", value).apply()
        }

    fun installing(release: UpdateRelease) {
        check(values.edit().putLong("installingVersion", release.versionCode)
            .putString("status", context.appText("android.update.installing", "version" to release.versionName)).commit())
    }

    fun confirmInstalledUpdate(version: Long, name: String) {
        val pending = values.getLong("installingVersion", 0)
        if (pending != 0L && version >= pending) {
            values.edit().remove("installingVersion").putString("status", context.appText("android.update.installed", "version" to name)).apply()
        }
    }

    var status: String
        get() = values.getString("status", null) ?: context.appText("android.update.notChecked")
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
                prefs.status = applicationContext.appText("android.update.releaseOnly")
                return@withLock Result.failure()
            }
            if (!manual && !prefs.enabled) return@withLock Result.success()
            if (!awaitShizukuReady()) {
                prefs.status = applicationContext.appText("android.update.shizukuRequired")
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
                prefs.status = applicationContext.appText("android.update.checking")
                val release = fetchUpdateRelease(http)
                val current =
                    applicationContext.packageManager.installed(applicationContext.packageName)
                if (release.versionCode <= current.version()) {
                    prefs.status = applicationContext.appText("android.update.upToDate", "version" to BuildConfig.VERSION_NAME)
                    return@withLock Result.success()
                }
                prefs.status = applicationContext.appText("android.update.downloading", "version" to release.versionName)
                val digest = MessageDigest.getInstance("SHA-256")
                var total = 0L
                http.prepareGet(release.url).execute { response ->
                    checkUpdate(response.status.value == 200, "android.update.apkHttpError", "status" to response.status.value.toString())
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
                                requireUpdate(total <= release.size && total <= MAX_APK_BYTES, "android.update.apkSizeMismatch")
                                digest.update(buffer, 0, count)
                                output.write(buffer, 0, count)
                            }
                        }
                    }
                }
                requireUpdate(
                    total == release.size &&
                        digest.digest().joinToString("") { "%02x".format(it) } == release.sha256,
                    "android.update.apkChecksumMismatch",
                )
                val archive =
                    applicationContext.packageManager.archive(apk) ?: throw UpdateStateException("android.update.apkUnreadable")
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
                prefs.installing(release)
                val result = ShizukuInstaller(applicationContext).install(apk)
                checkUpdate(result == "Success", "android.update.installFailed", "detail" to result.removePrefix(INSTALL_FAILED_PREFIX))
                prefs.confirmInstalledUpdate(release.versionCode, release.versionName)
                Result.success()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                prefs.status = applicationContext.appText(
                    "android.update.failed",
                    "reason" to applicationContext.updateFailureText(failure).take(1000),
                )
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
