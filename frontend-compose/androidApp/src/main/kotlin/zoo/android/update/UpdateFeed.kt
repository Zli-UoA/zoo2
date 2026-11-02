package zoo.android.update

import java.net.URI
import java.security.MessageDigest
import kotlinx.serialization.json.*

internal const val MAX_APK_BYTES = 200L * 1024 * 1024
internal const val UPDATE_BASE = "https://github.com/Zli-UoA/zoo2/releases/download/android-main/"
internal const val UPDATE_FEED = "${UPDATE_BASE}update.json"

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
            require(text("channel") == "main" && text("packageName") == "works.zli.zoo") {
                "更新チャンネルが一致しません"
            }
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
            require(
                url.startsWith(UPDATE_BASE) &&
                    Regex("zoo-android-main-[a-f0-9]{40}\\.apk")
                        .matches(url.removePrefix(UPDATE_BASE))
            ) {
                "更新元が一致しません"
            }
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
    require(archivePackage == expectedPackage) { "別のアプリの APK です" }
    require(archiveVersion == release.versionCode && archiveVersion > installedVersion) {
        "APK のバージョンが一致しません"
    }
    require(installedSigners.isNotEmpty() && installedSigners == archiveSigners) {
        "署名鍵が一致しません。現在のアプリと同じ鍵で署名された更新が必要です"
    }
}
