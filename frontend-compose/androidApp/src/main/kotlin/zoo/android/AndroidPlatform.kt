package zoo.android

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.runtime.*
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

internal sealed interface AndroidNavigationRequest {
    data object Back : AndroidNavigationRequest
    data object Exit : AndroidNavigationRequest
    data object Logout : AndroidNavigationRequest
    data class Navigate(val path: String) : AndroidNavigationRequest
    data class Replace(val path: String) : AndroidNavigationRequest
}

internal fun androidCopyUrl(base: String, payload: String): String {
    val url = if (payload.startsWith('/')) endpoint(base, payload) else payload
    require(Url(url).protocol in listOf(URLProtocol.HTTPS, URLProtocol.HTTP))
    return url
}

internal class AndroidNavigationState(initialPath: String) {
    private val history = mutableListOf(initialPath)

    var noteDirty: Boolean = false
        private set
    var pendingRequest: AndroidNavigationRequest? = null
        private set
    val path: String get() = history.last()
    val historyIndex: Int get() = history.lastIndex

    fun setNoteDirty(value: Boolean) {
        noteDirty = value
    }

    fun navigate(path: String) = request(AndroidNavigationRequest.Navigate(path))

    fun replace(path: String) = request(AndroidNavigationRequest.Replace(path))

    fun requestLogout() = request(AndroidNavigationRequest.Logout)

    fun back(): Boolean {
        if (history.size < 2) {
            if (!noteDirty) return false
            request(AndroidNavigationRequest.Exit)
            return true
        }
        request(AndroidNavigationRequest.Back)
        return true
    }

    fun cancelPendingRequest() {
        pendingRequest = null
    }

    fun confirmDiscard(): AndroidNavigationRequest? {
        val request = pendingRequest ?: return null
        pendingRequest = null
        noteDirty = false
        apply(request)
        return request
    }

    private fun request(request: AndroidNavigationRequest) {
        if (noteDirty) pendingRequest = request else apply(request)
    }

    private fun apply(request: AndroidNavigationRequest) {
        when (request) {
            AndroidNavigationRequest.Back -> if (history.size > 1) history.removeAt(history.lastIndex)
            AndroidNavigationRequest.Exit -> Unit
            AndroidNavigationRequest.Logout -> Unit
            is AndroidNavigationRequest.Navigate -> history.add(request.path)
            is AndroidNavigationRequest.Replace -> history[history.lastIndex] = request.path
        }
    }
}

internal class AndroidPlatform(
    private val activity: Activity,
    private val http: HttpClient,
    private val auth: DeviceAuth,
    private val base: String,
    private val userId: String,
    val logout: () -> Unit,
    private val chooseFiles: suspend () -> List<Uri>,
) : Platform {
    var env by mutableStateOf(Environment(path = "/home", touch = true, oidc = true))
        private set

    private val navigation = AndroidNavigationState(env.path)
    var pendingNavigation by mutableStateOf<AndroidNavigationRequest?>(null)
        private set
    private val prefs = activity.getSharedPreferences("preferences", 0)
    private val files = java.util.concurrent.ConcurrentHashMap<String, Uri>()
    private val cache =
        java.io.File(activity.cacheDir, "accounts/${hash("$base:$userId")}").apply { mkdirs() }
    private val dictionaries = mutableMapOf<String, Row>()

    private fun hash(value: String) =
        MessageDigest.getInstance("SHA-256").digest(value.toByteArray()).joinToString("") {
            "%02x".format(it)
        }

    override suspend fun headers() = auth.headers()

    override suspend fun readCache(key: String): String? =
        withContext(Dispatchers.IO) { cache.resolve(hash(key)).takeIf { it.exists() }?.readText() }

    override suspend fun writeCache(key: String, value: String) =
        withContext(Dispatchers.IO) {
            val target = cache.resolve(hash(key))
            val pending = cache.resolve("${hash(key)}.tmp")
            pending.writeText(value)
            check(pending.renameTo(target))
        }

    fun clear() {
        cache.deleteRecursively()
        files.clear()
    }

    override fun uuid() = UUID.randomUUID().toString()

    override fun now() = Instant.now().toString()

    override fun navigate(path: String) {
        navigation.navigate(path)
        syncNavigation()
    }

    override fun replace(path: String) {
        navigation.replace(path)
        syncNavigation()
    }

    fun back(): Boolean {
        val handled = navigation.back()
        syncNavigation()
        return handled
    }

    fun cancelPendingNavigation() {
        navigation.cancelPendingRequest()
        syncNavigation()
    }

    fun confirmDiscardAndNavigate(): Boolean {
        val request = navigation.confirmDiscard()
        syncNavigation()
        if (request == AndroidNavigationRequest.Logout) {
            try { logout() } finally { clear() }
        }
        return request == AndroidNavigationRequest.Exit
    }

    private fun syncNavigation() {
        env = env.copy(
            path = navigation.path,
            historyState = zoo.obj("__TSR_index" to navigation.historyIndex),
        )
        pendingNavigation = navigation.pendingRequest
    }

    fun online(value: Boolean) {
        env = env.copy(online = value)
    }

    fun visible(value: Boolean) {
        env = env.copy(visible = value)
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
        var value: JsonElement? = dictionary
        key.split('.').forEach { value = (value as? JsonObject)?.get(it) }
        var text = (value as? JsonPrimitive)?.content ?: key
        args.forEach { (name, replacement) ->
            text = text.replace("{{$name}}", replacement).replace("{$name}", replacement)
        }
        return text
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
            name.startsWith("open:") -> open(name.removePrefix("open:"))
            name.startsWith("theme:") -> {
                val theme = name.removePrefix("theme:")
                env =
                    env.copy(
                        theme = theme,
                        dark =
                            theme == "dark" ||
                                theme == "system" &&
                                    (activity.resources.configuration.uiMode and 48) == 32,
                    )
            }
            name.startsWith("locale:") -> env = env.copy(locale = name.removePrefix("locale:"))
            name.startsWith("fileRelease:") -> files.remove(name.removePrefix("fileRelease:"))
        }
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

    override suspend fun image(url: String): ByteArray {
        require(Url(url).protocol == URLProtocol.HTTPS)
        val response = http.get(url) // Public client: never attach account headers to media.
        check(response.status.value in 200..299)
        return response.readRawBytes()
    }

    override suspend fun invoke(name: String, payload: String): String =
        when (name) {
            "logout" -> {
                if (navigation.noteDirty) {
                    navigation.requestLogout()
                    syncNavigation()
                } else {
                    try { logout() } finally { clear() }
                }
                "{}"
            }
            "copy" -> {
                copyToClipboard(payload)
                "{}"
            }
            "copyUrl" -> {
                copyToClipboard(androidCopyUrl(base, payload))
                "{}"
            }
            "noteDirty" -> {
                navigation.setNoteDirty(payload == "true")
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
            "attachmentDownload" -> {
                open(json.parseToJsonElement(payload).jsonObject.text("url"))
                "{}"
            }
            "pickFiles" -> {
                val selected = chooseFiles().take(10)
                val rows =
                    withContext(Dispatchers.IO) {
                        selected.map { uri ->
                            val id = uuid()
                            files[id] = uri
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
                            )
                        }
                    }
                buildJsonObject {
                        put("origin", "pick")
                        put("files", JsonArray(rows))
                    }
                    .toString()
            }
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

    private fun copyToClipboard(value: String) {
        activity.getSystemService(ClipboardManager::class.java)
            .setPrimaryClip(ClipData.newPlainText("Zoo", value))
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
