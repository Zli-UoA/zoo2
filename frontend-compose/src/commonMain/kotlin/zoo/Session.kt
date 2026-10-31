package zoo

import androidx.compose.runtime.*
import io.ktor.client.*
import io.ktor.client.call.*
import io.ktor.client.request.*
import io.ktor.http.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*

class Session(val repo: Repository, val scope: CoroutineScope, initial: Environment) {
    val api
        get() = repo.api

    val platform
        get() = repo.platform

    val config
        get() = repo.config

    var env by mutableStateOf(initial)
    val overlays = mutableStateListOf<OverlaySlot>()
    var dialog by mutableStateOf<String?>(null)
    var thread by mutableStateOf<String?>(null)
    var membersOpen by mutableStateOf(false)
    var pane by mutableStateOf("sidebar")
    var toast by mutableStateOf<String?>(null)
    var pending by mutableStateOf<Set<String>>(emptySet())
    var unread by mutableStateOf<Set<String>>(emptySet())
    var admins by mutableStateOf<Set<String>>(emptySet())
    var surfaces by mutableStateOf<List<Row>>(emptyList())
    var capabilities by mutableStateOf<JsonObject>(JsonObject(emptyMap()))
    var target by mutableStateOf<String?>(null)
    private val assetClient = HttpClient()
    val assets = mutableMapOf<String, JsonElement>()
    val channelId: String?
        get() =
            env.path
                .substringBefore('?')
                .substringBefore('#')
                .takeIf { it.startsWith("/channels/") }
                ?.removePrefix("/channels/")
                ?.decodeURLPart()

    val query: Parameters
        get() = Url("https://zoo.invalid${env.path}").parameters

    fun t(key: String, vararg args: Pair<String, String>): String {
        env.locale
        return platform.translate(key, args.toMap())
    }

    fun dayLabel(timestamp: String): String {
        val value = platform.day(timestamp)
        val now = platform.now()
        return when (value) {
            platform.day(now) -> t("dates.today")
            platform.previousDay(now) -> t("dates.yesterday")
            else -> value
        }
    }

    private var lastEscape: kotlin.time.TimeMark? = null

    fun escape() {
        if (lastEscape?.elapsedNow()?.inWholeMilliseconds?.let { it < 100 } == true) return
        lastEscape = kotlin.time.TimeSource.Monotonic.markNow()
        overlays.lastOrNull()?.close?.invoke()
    }

    fun navigate(path: String) = platform.navigate(path)

    fun run(key: String = platform.uuid(), task: suspend () -> Unit) {
        if (key in pending) return
        pending = pending + key
        scope.launch {
            try {
                task()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast =
                    t(
                        when ((e as? ApiException)?.status) {
                            400 -> "common.errors.badRequest"
                            401 -> "common.errors.unauthorized"
                            403 -> "common.errors.forbidden"
                            404 -> "common.errors.notFound"
                            409 -> "common.errors.conflict"
                            429 -> "common.errors.rateLimited"
                            else -> "common.errors.network"
                        }
                    )
            } finally {
                pending = pending - key
            }
        }
    }

    suspend fun asset(path: String): JsonElement =
        assets[path]
            ?: json
                .parseToJsonElement(
                    assetClient.get(endpoint(config.assetsBaseUrl, path)).body<String>()
                )
                .also { assets[path] = it }

    suspend fun image(url: String): ByteArray =
        assetClient
            .get(if (url.startsWith("/")) endpoint(config.assetsBaseUrl, url) else url)
            .body()

    fun update(next: Environment) {
        val old = env
        env = next
        if (old.path.substringBefore('?') != next.path.substringBefore('?')) {
            thread = null
            pane =
                if (channelId != null || next.path.startsWith("/notifications")) "chat"
                else "sidebar"
        }
        if (!old.online && next.online) repo.refresh()
    }

    fun start() {
        pane = if (channelId != null || env.path.startsWith("/notifications")) "chat" else "sidebar"
        scope.launch {
            while (isActive) {
                if (env.online && env.visible) {
                    try {
                        unread =
                            api.request("/api/channel-read-states")
                                .rows()
                                .filter { it.flag("has_unread") }
                                .map { it.text("channel_id") }
                                .toSet()
                        capabilities =
                            json.parseToJsonElement(platform.invoke("capabilities")).jsonObject
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {}
                }
                delay(10_000)
            }
        }
        scope.launch {
            try {
                admins =
                    api.request("/api/users/admins")
                        .jsonObject["user_ids"]
                        ?.jsonArray
                        ?.map { it.jsonPrimitive.content }
                        ?.toSet()
                        .orEmpty()
            } catch (_: Exception) {}
        }
        scope.launch {
            while (isActive) {
                if (env.online && env.visible) {
                    try {
                        api.request("/api/presence/heartbeat", "POST", obj())
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {}
                }
                delay(30_000)
            }
        }
        scope.launch {
            while (isActive) {
                if (env.online && env.visible) {
                    try {
                        surfaces = api.request("/api/app-surfaces").rows()
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {}
                }
                delay(2_000)
            }
        }
    }

    suspend fun send(channel: String, content: String, parent: String?): Boolean {
        val id = platform.uuid()
        val row = buildJsonObject {
            put("id", id)
            put("channel_id", channel)
            put("user_id", config.userId)
            put("content", content)
            put("created_at", platform.now())
            put("parent_id", parent?.let(::JsonPrimitive) ?: JsonNull)
            put("deleted_at", JsonNull)
            put("updated_at", JsonNull)
        }
        val body = buildJsonObject {
            put("id", id)
            put("channel_id", channel)
            put("content", content)
            parent?.let { put("parent_id", it) }
        }
        return try {
            repo.messages(channel).write(api, "/api/messages", "POST", body, row)
            true
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            toast = t("common.writeError")
            false
        }
    }

    fun openThread(id: String, wide: Boolean) {
        thread = id
        if (wide) membersOpen = false
    }

    fun openMembers(wide: Boolean) {
        membersOpen = !membersOpen
        if (membersOpen && wide) thread = null
    }

    fun close() {
        assetClient.close()
        repo.close()
    }
}

fun String.decodeURLPart(): String = decodeURLQueryComponent()
