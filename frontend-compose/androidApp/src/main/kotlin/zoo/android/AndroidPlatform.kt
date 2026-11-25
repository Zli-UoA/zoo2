package zoo.android

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.asImageBitmap
import io.ktor.client.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*
import java.security.MessageDigest
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.UUID
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import zoo.*

internal class AndroidPlatform(
    private val activity: Activity,
    private val http: HttpClient,
    private val auth: DeviceAuth,
    private val base: String,
    private val userId: String,
    val logout: () -> Unit,
    private val chooseFiles: suspend (Boolean) -> List<Uri>,
    private val recentPhotos: RecentPhotos,
    private val requestRecentPhotos: suspend () -> Unit,
    /**
     * 鍵バックアップに送れていない鍵があるときの、ログアウトの確認（docs/e2ee.md の「ログアウトと
     * 端末の削除」）。`count` が負なら数えられなかった。ログアウトしてよければ真を返す。
     */
    private val confirmUnbackedKeys: suspend (count: Int) -> Boolean,
) : Platform {
    /** E2EE の端末（#359）。ログインしたあとに設定する。 */
    override var messageEncryption: zoo.MessageEncryption? = null
    var reauthenticate: (suspend () -> String)? = null
    var passkey: zoo.crypto.UnlockMethod.Passkey? = null
    /** ログインに使う OIDC の issuer（失効の画面の AuthZ へのリンクに使う）。 */
    var issuer: String? = null
    var e2ee: zoo.crypto.E2eeDeviceManager? = null

    private val prefs = activity.getSharedPreferences("preferences", 0)
    private val savedTheme = prefs.getString("theme", "system") ?: "system"
    private fun reducedMotion() = android.provider.Settings.Global.getFloat(
        activity.contentResolver, android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f,
    ) == 0f

    var env by mutableStateOf(Environment(path = "/home", touch = true, oidc = true,
        theme = savedTheme,
        locale = prefs.getString("locale", "ja") ?: "ja",
        dark = savedTheme == "dark" || savedTheme == "system" && (activity.resources.configuration.uiMode and 48) == 32,
        reducedMotion = reducedMotion(),
        swipeReply = prefs.getBoolean("swipe-reply", true),
        operationHaptics = prefs.getBoolean("operation-haptics", true),
    ))
        private set

    private val history = NavigationHistory(env.path)
    private val files = java.util.concurrent.ConcurrentHashMap<String, Uri>()
    private val cacheDirectory = java.io.File(activity.cacheDir, "accounts/${hash("$base:$userId")}")
    private val cache = AccountCache(cacheDirectory)
    private val encryptionHistory = AccountCache(java.io.File(activity.noBackupFilesDir, "e2ee-channel-history/${hash("$base:$userId")}"))
    private val images = AndroidImageCache(cacheDirectory.resolve("images"))
    private val dictionaries = mutableMapOf<String, Row>()

    private fun hash(value: String) =
        MessageDigest.getInstance("SHA-256").digest(value.toByteArray()).joinToString("") {
            "%02x".format(it)
        }

    override fun imageFrames(bytes: ByteArray): kotlinx.coroutines.flow.Flow<androidx.compose.ui.graphics.ImageBitmap> =
        kotlinx.coroutines.flow.flow {
            emit(withContext(Dispatchers.Default) { decodeOrientedBitmap(bytes).asImageBitmap() })
        }

    override suspend fun headers() = auth.headers()

    override suspend fun readEncryptionHistory(key: String): String? = withContext(Dispatchers.IO) { encryptionHistory.read(key) }

    override suspend fun writeEncryptionHistory(key: String, value: String) = withContext(Dispatchers.IO) { encryptionHistory.write(key, value) }

    override suspend fun readCache(key: String): String? =
        withContext(Dispatchers.IO) { cache.read(key) }

    override suspend fun writeCache(key: String, value: String) =
        withContext(Dispatchers.IO) {
            cache.write(key, value)
        }

    override suspend fun removeCache(key: String) = withContext(Dispatchers.IO) { cache.delete(key) }

    fun close() { images.close() }

    private suspend fun clear() = withContext(NonCancellable + Dispatchers.IO) {
        // logout closes the session scope; deletion must still finish, off the UI thread.
        close()
        cache.clear()
        encryptionHistory.clear()
        files.clear()
    }

    override fun uuid() = UUID.randomUUID().toString()

    override fun now() = Instant.now().toString()

    override fun navigate(path: String) {
        applyNavigation(history.navigate(path))
    }

    override fun navigate(path: String, state: Row) {
        applyNavigation(history.navigate(path, state))
    }

    private fun applyNavigation(entry: NavigationHistory.Entry) {
        env = env.copy(path = entry.path, historyState = entry.state)
    }

    override fun replace(path: String) {
        applyNavigation(history.replace(path))
    }

    fun back(): Boolean {
        val entry = history.back() ?: return false
        applyNavigation(entry)
        return true
    }

    fun online(value: Boolean) {
        env = env.copy(online = value)
    }

    fun protectScreen(): () -> Unit {
        val flag = android.view.WindowManager.LayoutParams.FLAG_SECURE
        val alreadyProtected = activity.window.attributes.flags and flag != 0
        if (!alreadyProtected) activity.window.addFlags(flag)
        return { if (!alreadyProtected) activity.window.clearFlags(flag) }
    }

    fun visible(value: Boolean) {
        env = env.copy(visible = value, reducedMotion = reducedMotion())
    }

    fun systemDark(value: Boolean) {
        if (env.theme == "system") env = env.copy(dark = value)
    }

    override fun translate(key: String, args: Map<String, String>): String {
        val dictionary =
            dictionaries.getOrPut(env.locale) {
                json
                    .parseToJsonElement(
                        activity.assets.open("locales/${env.locale}.json").bufferedReader().use {
                            it.readText()
                        }
                    )
                    .jsonObject
            }
        return translateDictionary(dictionary, env.locale, key, args)
    }

    private fun format(timestamp: String, pattern: String): String =
        try {
            val instant =
                Instant.ofEpochMilli(timestampMillis(timestamp) ?: throw IllegalArgumentException())
            DateTimeFormatter.ofPattern(pattern, java.util.Locale.forLanguageTag(env.locale))
                .withZone(ZoneId.systemDefault())
                .format(instant)
        } catch (_: Exception) {
            timestamp
        }

    override fun time(timestamp: String) = format(timestamp, "HH:mm")

    override fun relativeTimestamp(timestamp: String): String =
        androidRelativeTimestamp(timestamp, System.currentTimeMillis(), env.locale) ?: fullTimestamp(timestamp)

    override fun day(timestamp: String) = format(timestamp, "yyyy/MM/dd")

    override fun previousDay(timestamp: String): String {
        val millis = timestampMillis(timestamp) ?: return timestamp
        return day(
            Instant.ofEpochMilli(millis)
                .atZone(ZoneId.systemDefault())
                .minusDays(1)
                .toInstant()
                .toString()
        )
    }

    override fun action(name: String) {
        when {
            name == "back" -> back()
            name == "haptic:longPress" && env.operationHaptics ->
                activity.window.decorView.performHapticFeedback(android.view.HapticFeedbackConstants.LONG_PRESS)
            name == "haptic:threshold" && env.operationHaptics ->
                activity.window.decorView.performHapticFeedback(android.view.HapticFeedbackConstants.CLOCK_TICK)
            name == "toggleSwipeReply" -> updateInteraction("swipe-reply", !env.swipeReply)
            name == "toggleOperationHaptics" -> updateInteraction("operation-haptics", !env.operationHaptics)
            name.startsWith("open:") -> open(name.removePrefix("open:"))
            name.startsWith("theme:") -> {
                val theme = name.removePrefix("theme:")
                prefs.edit().putString("theme", theme).apply()
                env =
                    env.copy(
                        theme = theme,
                        dark =
                            theme == "dark" ||
                                theme == "system" &&
                                    (activity.resources.configuration.uiMode and 48) == 32,
                    )
            }
            name.startsWith("locale:") -> {
                val locale = name.removePrefix("locale:")
                prefs.edit().putString("locale", locale).apply()
                env = env.copy(locale = locale)
            }
            name.startsWith("fileRelease:") -> files.remove(name.removePrefix("fileRelease:"))
        }
    }

    private fun updateInteraction(key: String, enabled: Boolean) {
        prefs.edit().putBoolean(key, enabled).apply()
        env = if (key == "swipe-reply") env.copy(swipeReply = enabled) else env.copy(operationHaptics = enabled)
    }

    private fun open(url: String) {
        val absolute = if (url.startsWith('/')) endpoint(base, url) else url
        require(Url(absolute).protocol in listOf(URLProtocol.HTTPS, URLProtocol.HTTP))
        activity.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(absolute)))
    }

    override suspend fun rasterizeSvg(svg: String, width: Int, height: Int): ByteArray =
        withContext(Dispatchers.Default) {
            val bitmap =
                android.graphics.Bitmap.createBitmap(
                    width,
                    height,
                    android.graphics.Bitmap.Config.ARGB_8888,
                )
            try {
                com.caverock.androidsvg.SVG.getFromString(svg)
                    .renderToCanvas(android.graphics.Canvas(bitmap))
                java.io.ByteArrayOutputStream().use { output ->
                    bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
                    output.toByteArray()
                }
            } finally {
                bitmap.recycle()
            }
        }

    override suspend fun image(url: String): ByteArray = images.image(url)

    override suspend fun invoke(name: String, payload: String): String =
        when (name) {
            "logout" -> {
                val manager = e2ee
                // 件数を数える前に to-device の処理を止める（数えたあとで増えた鍵を、確かめずに消さないため）。
                val count = manager?.prepareLogout() ?: 0
                // 終わっていない失効があれば、進み具合が消えることを先に確かめる（Web の `logout.revocation*`）。
                val revocationPending = manager?.revocationPendingForLogout() == true
                if (revocationPending && !confirmUnbackedKeys(REVOCATION_PENDING_PROMPT) || count != 0 && !confirmUnbackedKeys(count)) {
                    // 取りやめたら、to-device の処理に戻る。
                    manager?.cancelLogout()
                } else {
                    // 端末の削除 API にトークンが要るので、認証情報を消す前に行う。
                    manager?.destroyForLogout()
                    try { logout() } finally { clear() }
                }
                "{}"
            }
            "copy" -> {
                (activity.getSystemService(ClipboardManager::class.java)).setPrimaryClip(
                    ClipData.newPlainText("Zoo", payload)
                )
                "{}"
            }
            "capabilities" -> "{\"pushSupported\":false,\"installAvailable\":false}"
            "storageRead" -> prefs.getString("$base:$userId:$payload", "").orEmpty()
            "storageWrite" -> {
                val data = json.parseToJsonElement(payload).jsonObject
                prefs
                    .edit()
                    .putString("$base:$userId:${data.text("key")}", data.text("value"))
                    .apply()
                "{}"
            }
            "publicGet" -> {
                val url = json.parseToJsonElement(payload).jsonObject.text("url")
                require(Url(url).protocol == URLProtocol.HTTPS)
                val response = http.get(url)
                check(response.status.value in 200..299)
                zoo.obj(
                        "body" to response.bodyAsText(),
                        "contentType" to response.headers["Content-Type"].orEmpty(),
                    )
                    .toString()
            }
            "redirect" -> {
                open(payload)
                "{}"
            }
            "previewSave" -> {
                val data = json.parseToJsonElement(payload).jsonObject
                require(data.text("base64").length <= 24 * 1024 * 1024)
                val bytes = android.util.Base64.decode(data.text("base64"), android.util.Base64.DEFAULT)
                require(bytes.size <= 16 * 1024 * 1024)
                shareFile(bytes, data.text("name"), data.text("type"))
                "{}"
            }
            "attachmentDownload" -> {
                open(json.parseToJsonElement(payload).jsonObject.text("url"))
                "{}"
            }
            "recentPhotos" -> recentPhotos.list()
            "requestRecentPhotos" -> {
                requestRecentPhotos()
                recentPhotos.list()
            }
            "pickRecentPhoto" -> registerFiles(listOf(recentPhotos.select(payload.toLong())), mediaStoreId = payload)
            "pickFiles" -> registerFiles(chooseFiles(json.parseToJsonElement(payload).jsonObject.flag("photos")).take(10))
            "fileRead" ->
                android.util.Base64.encodeToString(readFile(payload), android.util.Base64.NO_WRAP)
            "filePut" -> {
                val data = json.parseToJsonElement(payload).jsonObject
                val url = data.text("url")
                require(Url(url).protocol == URLProtocol.HTTPS)
                val response =
                    http.put(url) {
                        contentType(ContentType.parse(data.text("type")))
                        setBody(readFile(data.text("id")))
                    }
                check(response.status.value in 200..299)
                "{}"
            }
            "refreshUser" -> "{}" // Profile updates are rendered by the shared users Shape.
            else -> error("Android では未対応の操作です: $name")
        }

    /**
     * 端末の非公開キャッシュにファイルを書き、共有の画面で開く（プレビューの保存と、復号した暗号化の添付。
     * 暗号化の添付は平文を文字列にしないで、そのまま書く）。キャッシュは 64 MiB と 1 日までに保つ。
     */
    suspend fun shareFile(bytes: ByteArray, name: String, type: String) {
        require(bytes.size <= 64 * 1024 * 1024)
        val target = withContext(Dispatchers.IO) {
            val directory = java.io.File(activity.cacheDir, "preview-files").apply { mkdirs() }
            // Share only user-selected files, retaining at most 64 MiB in the private cache.
            val existing = directory.listFiles().orEmpty().sortedBy { it.lastModified() }.toMutableList()
            var total = existing.sumOf { it.length() }
            while (existing.isNotEmpty() && (total + bytes.size > 64 * 1024 * 1024 || existing.first().lastModified() < System.currentTimeMillis() - 86_400_000)) {
                val old = existing.removeAt(0); total -= old.length(); old.delete()
            }
            val safeName = sharedFileName(name.substringAfterLast('/').substringAfterLast('\\').replace(Regex("[\\p{Cntrl}]"), "_"))
            java.io.File(directory, "${UUID.randomUUID()}-$safeName").apply { writeBytes(bytes) }
        }
        withContext(Dispatchers.Main) {
            val uri = androidx.core.content.FileProvider.getUriForFile(activity, "${activity.packageName}.preview-files", target)
            val share = Intent(Intent.ACTION_SEND).apply {
                this.type = type.ifEmpty { "application/octet-stream" }
                putExtra(Intent.EXTRA_STREAM, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            activity.startActivity(Intent.createChooser(share, name))
        }
    }

    private suspend fun registerFiles(selected: List<Uri>, mediaStoreId: String? = null): String {
        val rows =
            withContext(Dispatchers.IO) {
                selected.map { uri ->
                    val id = uuid()
                    var name = "attachment"
                    var size = 0L
                    activity.contentResolver
                        .query(
                            uri,
                            arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE),
                            null,
                            null,
                            null,
                        )
                        ?.use { cursor ->
                            if (cursor.moveToFirst()) {
                                name = cursor.getString(0)
                                size = cursor.getLong(1)
                            }
                        }
                    zoo.obj(
                        "id" to id,
                        "name" to name,
                        "size" to size,
                        "type" to activity.contentResolver.getType(uri).orEmpty(),
                        "media_store_id" to mediaStoreId,
                    )
                }
            }
        val response = buildJsonObject {
                put("origin", "pick")
                put("files", JsonArray(rows))
            }
            .toString()
        rows.zip(selected).forEach { (row, uri) -> files[row.text("id")] = uri }
        return response
    }

    private suspend fun readFile(id: String): ByteArray =
        withContext(Dispatchers.IO) {
            val uri = files[id] ?: error("ファイルを選び直してください")
            activity.contentResolver.openInputStream(uri)!!.use { stream ->
                val output = java.io.ByteArrayOutputStream()
                val buffer = ByteArray(8192)
                while (true) {
                    val count = stream.read(buffer)
                    if (count < 0) break
                    require(output.size() + count <= 50 * 1024 * 1024) { "添付ファイルは50 MiB以内です" }
                    output.write(buffer, 0, count)
                }
                output.toByteArray()
            }
        }
}

/**
 * 共有のために書くファイルの名前（UUID とハイフンの 37 バイトを足して、ファイル名の上限 255 バイトに収める）。
 * 短い名前はそのまま使い、長い名前は文字の途中で切らずに縮め、拡張子は残す。
 */
internal fun sharedFileName(name: String, maxBytes: Int = 255 - 37): String {
    fun bytes(value: String) = value.toByteArray(Charsets.UTF_8).size
    val clean = name.ifEmpty { "attachment" }
    if (bytes(clean) <= maxBytes) return clean
    val dot = clean.lastIndexOf('.')
    val extension = if (dot > 0 && bytes(clean.substring(dot)) <= 32) clean.substring(dot) else ""
    val budget = maxBytes - bytes(extension)
    val stem = StringBuilder()
    var used = 0
    var index = 0
    // 拡張子を残せないときは、縮めた末尾が新しい拡張子（`.exe` など）にならないよう、ドットを `_` にする。
    val base = clean.substring(0, clean.length - extension.length).let { if (extension.isEmpty()) it.replace('.', '_') else it }
    while (index < base.length) {
        val end = base.offsetByCodePoints(index, 1)
        val size = bytes(base.substring(index, end))
        if (used + size > budget) break
        stem.append(base, index, end)
        used += size
        index = end
    }
    return (stem.toString() + extension).ifEmpty { "attachment" }
}

/** ログアウトの確認で、終わっていない失効があることを知らせる（鍵の件数の代わりに渡す値）。 */
internal const val REVOCATION_PENDING_PROMPT = Int.MIN_VALUE
