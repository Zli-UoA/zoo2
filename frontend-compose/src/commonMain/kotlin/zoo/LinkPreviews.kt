package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import io.ktor.http.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*
import org.jetbrains.compose.resources.decodeToImageBitmap

fun publicUrl(raw: String): Url? =
    try {
        Url(raw)
            .takeIf { raw.startsWith("http://", true) || raw.startsWith("https://", true) }
            ?.takeIf {
                raw.length <= 4096 &&
                    it.protocol.name in listOf("http", "https") &&
                    it.host.isNotEmpty() &&
                    it.user == null &&
                    it.password == null
            }
            ?.let { url ->
                if (url.encodedPath.isEmpty()) URLBuilder(url).apply { encodedPath = "/" }.build()
                else url
            }
    } catch (_: Exception) {
        null
    }

/** Only visible links create cards: code, plain and MFM silent links stay private. */
fun previewUrls(content: String): List<String> {
    val visible =
        content.replace(
            Regex(
                "```[\\s\\S]*?```|`[^`\\n]*`|<plain>[\\s\\S]*?</plain>|\\?\\[[^]\\n]*]\\([^\\s)]+\\)"
            ),
            " ",
        )
    return Regex("https?://[^\\s<>\\\"\\[\\]]+")
        .findAll(visible)
        .mapNotNull { publicUrl(it.value.trimEnd(')', ',', '.', '!', '?', '、', '。'))?.toString() }
        .distinct()
        .take(3)
        .toList()
}

fun proxyImageUrl(endpoint: String, image: String): String? {
    if (publicUrl(image) == null) return null
    val relative = endpoint.startsWith('/') && !endpoint.startsWith("//")
    val target =
        publicUrl(if (relative) "https://same-origin.invalid$endpoint" else endpoint) ?: return null
    if (target.protocol.name != "https" && target.host !in listOf("localhost", "127.0.0.1"))
        return null
    val builder =
        URLBuilder(target).apply {
            parameters["url"] = image
            parameters.remove("preview")
            parameters.remove("static")
        }
    return if (relative) builder.buildString().removePrefix("https://same-origin.invalid")
    else builder.buildString()
}

data class PreviewData(
    val title: String,
    val description: String,
    val image: String,
    val icon: String,
    val site: String,
)

fun parsePreview(data: Row, url: String) =
    PreviewData(
        data.text("title").trim().ifEmpty { Url(url).host }.take(300),
        data.text("description").take(1000),
        data.text("thumbnail"),
        data.text("icon"),
        data.text("sitename").trim().ifEmpty { Url(url).host }.take(300),
    )

class PreviewLoader(private val app: Session) {
    private val cache = mutableMapOf<String, PreviewData>()

    private data class Work(val request: Deferred<PreviewData?>, var users: Int = 0)

    private val pending = mutableMapOf<String, Work>()
    private var resolved: Pair<String, String>? = null
    val endpoints =
        flow {
                resolved?.let {
                    emit(it)
                    return@flow
                }
                var wait = 2000L
                while (currentCoroutineContext().isActive) {
                    try {
                        suspend fun runtime(path: String): String {
                            val value =
                                json
                                    .parseToJsonElement(
                                        app.platform.invoke(
                                            "publicGet",
                                            obj("url" to endpoint(app.config.assetsBaseUrl, path))
                                                .toString(),
                                        )
                                    )
                                    .jsonObject
                            return if (value.text("contentType").startsWith("text/plain"))
                                value.text("body").trim()
                            else ""
                        }
                        val next = coroutineScope {
                            val summaly = async { runtime("/summaly-endpoint") }
                            val media = async { runtime("/media-proxy-endpoint") }
                            summaly.await().ifEmpty { app.config.summalyUrl } to
                                media.await().ifEmpty { app.config.mediaProxyUrl }
                        }
                        resolved = next
                        emit(next)
                        break
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        emit(app.config.summalyUrl to app.config.mediaProxyUrl)
                        delay(wait)
                        wait = (wait * 2).coerceAtMost(30000)
                    }
                }
            }
            .stateIn(
                app.scope,
                SharingStarted.WhileSubscribed(0),
                app.config.summalyUrl to app.config.mediaProxyUrl,
            )

    suspend fun load(service: String, url: String): PreviewData? {
        val key = "$service:$url"
        cache[key]?.let {
            return it
        }
        val work =
            pending.getOrPut(key) {
                Work(
                    app.scope.async {
                        try {
                            val target =
                                publicUrl(
                                    if (service.startsWith('/') && !service.startsWith("//"))
                                        endpoint(app.config.assetsBaseUrl, service)
                                    else service
                                ) ?: return@async null
                            if (
                                target.protocol.name != "https" &&
                                    target.host !in listOf("localhost", "127.0.0.1")
                            )
                                return@async null
                            val path =
                                URLBuilder(target).apply { parameters["url"] = url }.buildString()
                            val response =
                                json
                                    .parseToJsonElement(
                                        app.platform.invoke(
                                            "publicGet",
                                            obj("url" to path).toString(),
                                        )
                                    )
                                    .jsonObject
                            parsePreview(
                                    json.parseToJsonElement(response.text("body")).jsonObject,
                                    url,
                                )
                                .also {
                                    if (cache.size >= 200) cache.remove(cache.keys.first())
                                    cache[key] = it
                                }
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {
                            null
                        }
                    }
                )
            }
        work.users++
        return try {
            work.request.await()
        } finally {
            work.users--
            if (work.users == 0) {
                if (!work.request.isCompleted) work.request.cancel()
                if (pending[key] === work) pending.remove(key)
            }
        }
    }
}

@Composable
fun LinkPreviews(app: Session, content: String) {
    val urls = remember(content) { previewUrls(content) }
    if (urls.isEmpty()) return
    val sources by app.previews.endpoints.collectAsState()
    sources
        .takeIf { it.first.isNotBlank() }
        ?.let { (service, media) ->
            urls.forEach { url ->
                var data by remember(url, service) { mutableStateOf<PreviewData?>(null) }
                LaunchedEffect(url, service, app.env.online) {
                    if (app.env.online) data = app.previews.load(service, url)
                }
                data?.let { preview -> PreviewCard(app, url, preview, media) }
            }
        }
}

@Composable
private fun PreviewCard(app: Session, url: String, preview: PreviewData, media: String) {
    val image = proxyImageUrl(media, preview.image)
    val icon = proxyImageUrl(media, preview.icon)
    var bitmap by remember(image) { mutableStateOf<ImageBitmap?>(null) }
    var iconBitmap by remember(icon) { mutableStateOf<ImageBitmap?>(null) }
    var imageComplete by remember(image) { mutableStateOf(image == null) }
    LaunchedEffect(image) {
        bitmap = null
        imageComplete = image == null
        if (image != null)
            try {
                bitmap = app.image(image).decodeToImageBitmap()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {} finally {
                imageComplete = true
            }
    }
    LaunchedEffect(icon) {
        iconBitmap = null
        if (icon != null)
            try {
                iconBitmap = app.image(icon).decodeToImageBitmap()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {}
    }
    val wide = bitmap?.let { it.width.toFloat() / it.height >= 1.5f } == true
    val text: @Composable () -> Unit = {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(preview.title, weight = FontWeight.SemiBold, maxLines = 2)
            if (preview.description.isNotEmpty())
                Text(preview.description, muted = true, maxLines = 2)
            Row(Modifier.padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                iconBitmap?.let {
                    Image(it, null, Modifier.size(16.dp), contentScale = ContentScale.Fit)
                }
                Text(preview.site, small = true, muted = true, maxLines = 1)
            }
        }
    }
    NativeLink(
        url,
        busy = !imageComplete,
        modifier =
            Modifier.widthIn(max = 384.dp)
                .fillMaxWidth()
                .padding(vertical = 8.dp)
                .clip(RoundedCornerShape(8.dp))
                .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp)),
    ) {
        if (wide)
            Column {
                bitmap?.let {
                    Image(
                        it,
                        null,
                        Modifier.fillMaxWidth().heightIn(max = 208.dp).aspectRatio(16f / 9),
                        contentScale = ContentScale.Crop,
                    )
                }
                text()
            }
        else
            Row(Modifier.height(IntrinsicSize.Min)) {
                bitmap?.let {
                    Image(
                        it,
                        null,
                        Modifier.width(if (LocalViewportWidth.current < 640f) 96.dp else 112.dp)
                            .fillMaxHeight()
                            .heightIn(min = 96.dp),
                        contentScale = ContentScale.Crop,
                    )
                }
                Box(Modifier.weight(1f)) { text() }
            }
    }
}
