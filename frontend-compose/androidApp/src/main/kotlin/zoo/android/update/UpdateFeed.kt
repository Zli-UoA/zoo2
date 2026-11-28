package zoo.android.update

import io.ktor.client.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.utils.io.core.readText
import io.ktor.utils.io.readRemaining
import java.net.URI
import java.security.MessageDigest
import kotlinx.serialization.json.*
import zoo.android.appText

internal const val MAX_APK_BYTES = 200L * 1024 * 1024
internal const val UPDATE_BASE = "https://github.com/Zli-UoA/zoo2/releases/download/android-main/"
internal const val UPDATE_FEED = "${UPDATE_BASE}update.json"

/** 利用者に見せる更新の失敗。メッセージは翻訳キーで、表示するときに [updateFailureText] で訳す。 */
internal interface UpdateFailureText {
    val key: String
    val args: List<Pair<String, String>>
}

internal class UpdateArgumentException(override val key: String, override val args: List<Pair<String, String>> = emptyList()) :
    IllegalArgumentException(key), UpdateFailureText

internal class UpdateStateException(override val key: String, override val args: List<Pair<String, String>> = emptyList()) :
    IllegalStateException(key), UpdateFailureText

internal fun requireUpdate(value: Boolean, key: String, vararg args: Pair<String, String>) {
    if (!value) throw UpdateArgumentException(key, args.toList())
}

internal fun checkUpdate(value: Boolean, key: String, vararg args: Pair<String, String>) {
    if (!value) throw UpdateStateException(key, args.toList())
}

internal fun android.content.Context.updateFailureText(failure: Throwable): String =
    (failure as? UpdateFailureText)?.let { appText(it.key, *it.args.toTypedArray()) }
        ?: failure.message.orEmpty()

internal data class UpdateRelease(
    val versionCode: Long,
    val versionName: String,
    val url: String,
    val sha256: String,
    val size: Long,
) {
    companion object {
        fun parse(raw: String): UpdateRelease {
            val data = Json.parseToJsonElement(raw).jsonObject
            fun text(key: String) = data.getValue(key).jsonPrimitive.content
            requireUpdate(text("channel") == "main" && text("packageName") == "works.zli.zoo", "android.update.channelMismatch")
            val url = text("url")
            val uri = URI(url)
            require(
                uri.scheme == "https" &&
                    uri.host == "github.com" &&
                    uri.userInfo == null &&
                    uri.port == -1 &&
                    uri.query == null &&
                    uri.fragment == null
            )
            requireUpdate(
                url.startsWith(UPDATE_BASE) &&
                    Regex("zoo-android-main-[a-f0-9]{40}\\.apk")
                        .matches(url.removePrefix(UPDATE_BASE)),
                "android.update.sourceMismatch",
            )
            val sha = text("sha256")
            require(Regex("[a-f0-9]{64}").matches(sha))
            val size = text("size").toLong()
            val version = text("versionCode").toLong()
            require(size in 1..MAX_APK_BYTES && version in 1..Int.MAX_VALUE.toLong())
            return UpdateRelease(version, text("versionName"), url, sha, size)
        }
    }
}

internal fun sha256(bytes: ByteArray): String =
    MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

internal fun requireMatchingApk(
    expectedPackage: String,
    installedVersion: Long,
    installedSigners: Set<String>,
    archivePackage: String?,
    archiveVersion: Long,
    archiveSigners: Set<String>,
    release: UpdateRelease,
) {
    requireUpdate(archivePackage == expectedPackage, "android.update.otherApp")
    requireUpdate(archiveVersion == release.versionCode && archiveVersion > installedVersion, "android.update.versionMismatch")
    requireUpdate(installedSigners.isNotEmpty() && installedSigners == archiveSigners, "android.update.signerMismatch")
}

internal suspend fun fetchUpdateRelease(http: HttpClient): UpdateRelease {
    val raw =
        http.prepareGet(UPDATE_FEED).execute { response ->
            checkUpdate(response.status.value == 200, "android.update.feedHttpError", "status" to response.status.value.toString())
            response.bodyAsChannel().readRemaining(16_385).readText()
        }
    requireUpdate(raw.toByteArray().size <= 16_384, "android.update.feedTooLarge")
    return UpdateRelease.parse(raw)
}
