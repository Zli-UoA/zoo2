package zoo

import io.ktor.http.*
import kotlin.time.TimeSource
import kotlin.time.TimeMark
import kotlin.time.Duration.Companion.minutes
import androidx.compose.runtime.mutableStateMapOf
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

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

fun publicUrlString(raw: String): String? = publicUrl(raw)?.toString()

/** Only visible links create cards: code, plain and MFM silent links stay private. */
fun previewUrls(content: String, excludeUrl: String? = null): List<String> {
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
        .filter { it != publicUrl(excludeUrl.orEmpty())?.toString() }
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

class PreviewLoader(private val app: Session, private val clock: TimeSource = TimeSource.Monotonic) {
    private data class Cached(val data: PreviewData, val storedAt: TimeMark)
    private val cache = mutableStateMapOf<String, Cached>()
    private val order = linkedSetOf<String>()

    fun cached(service: String, url: String): PreviewData? = cache["$service:$url"]?.data

    private fun store(key: String, data: PreviewData) {
        order.remove(key)
        order.add(key)
        cache[key] = Cached(data, clock.markNow())
        while (order.size > 256) cache.remove(order.first().also { order.remove(it) })
    }

    private data class Work(val request: Deferred<PreviewData?>, var users: Int = 0)

    private val pending = mutableMapOf<String, Work>()
    private val summalyEndpoint = runtimeEndpoint("/summaly-endpoint", app.config.summalyUrl)
    val mediaProxyEndpoint = runtimeEndpoint("/media-proxy-endpoint", app.config.mediaProxyUrl)
    val endpoints =
        summalyEndpoint.combine(mediaProxyEndpoint) { summaly, media -> summaly to media }
            .stateIn(
                app.scope,
                SharingStarted.WhileSubscribed(0),
                app.config.summalyUrl to app.config.mediaProxyUrl,
            )

    private fun runtimeEndpoint(path: String, fallback: String): StateFlow<String> {
        var resolved: String? = null
        return flow {
            resolved?.let {
                emit(it)
                return@flow
            }
            var wait = 2000L
            while (currentCoroutineContext().isActive) {
                if (!app.env.online || !app.env.visible) {
                    delay(wait)
                    continue
                }
                val configured = try {
                    val value = json.parseToJsonElement(
                        app.platform.invoke(
                            "publicGet",
                            obj("url" to endpoint(app.config.assetsBaseUrl, path)).toString(),
                        )
                    ).jsonObject
                    if (value.text("contentType").startsWith("text/plain"))
                        value.text("body").trim()
                    else ""
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    delay(wait)
                    wait = (wait * 2).coerceAtMost(30000)
                    continue
                }
                val value = configured.ifEmpty { fallback }
                resolved = value
                emit(value)
                break
            }
        }.stateIn(app.scope, SharingStarted.WhileSubscribed(0), fallback)
    }

    suspend fun load(service: String, url: String): PreviewData? {
        val key = "$service:$url"
        val previous = cache[key]
        if (previous != null && previous.storedAt.elapsedNow() < 5.minutes) {
            order.remove(key)
            order.add(key)
            return previous.data
        }
        val work =
            pending.getOrPut(key) {
                Work(
                    app.scope.async {
                        repeat(3) { attempt ->
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
                            return@async parsePreview(
                                    json.parseToJsonElement(response.text("body")).jsonObject,
                                    url,
                                )
                                .also {
                                    store(key, it)
                                }
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {
                            if (attempt < 2) delay(250L * (attempt + 1))
                        }
                        }
                        // Failed revalidation keeps the previous card and backs off.
                        previous?.data?.also { store(key, it) }
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
