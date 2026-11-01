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
    var appDetailId by mutableStateOf<String?>(null)
    var thread by mutableStateOf<String?>(null)
    private var membersPanelOpen by mutableStateOf(false)
    private var membersWide by mutableStateOf<Boolean?>(null)
    val membersRequested: Boolean
        get() = query["members"] == "true"

    var membersOpen: Boolean
        get() = if (membersWide == true) membersPanelOpen else membersRequested
        set(value) {
            setMembersOpen(value, membersWide == true)
        }

    var pane by
        mutableStateOf(
            if (
                initial.path.startsWith("/channels/") ||
                    initial.path.startsWith("/notifications") ||
                    isHomePath(initial.path)
            )
                "chat"
            else "sidebar"
        )
    private val toastController =
        NativeToastController(scope).also { it.setVisible(initial.visible) }
    val toastState: NativeToastState?
        get() = toastController.current

    var toast: String?
        get() = toastState?.message
        set(value) {
            if (value == null) dismissToast() else showToast(value, ToastKind.Error)
        }

    fun showToast(
        message: String,
        kind: ToastKind = ToastKind.Default,
        durationMillis: Long? = 4_000L,
    ) = toastController.show(message, kind, durationMillis)

    fun dismissToast(serial: Long? = null) = toastController.dismiss(serial)

    fun setToastHovered(serial: Long, hovered: Boolean) =
        toastController.setHovered(serial, hovered)

    var pending by mutableStateOf<Set<String>>(emptySet())
    var unread by mutableStateOf<Set<String>>(emptySet())
    var admins by mutableStateOf<Set<String>>(emptySet())
    var searchEngine by mutableStateOf("google")
    private val preferenceState = lazy { SearchPreferencesController(this, scope) }
    val searchPreferences
        get() = preferenceState.value

    var surfaces by mutableStateOf<List<Row>>(emptyList())
    var capabilities by mutableStateOf<JsonObject>(JsonObject(emptyMap()))
    var target by mutableStateOf<String?>(null)
    private val assetClient = HttpClient()
    val assets = mutableMapOf<String, JsonElement>()
    val activityPubNotifications by lazy { ActivityPubNotificationsState(this) }
    val previews = PreviewLoader(this)
    val attachmentLinks = AttachmentLinks(this)
    val channelId: String?
        get() =
            env.path
                .substringBefore('?')
                .substringBefore('#')
                .takeIf { it.startsWith("/channels/") }
                ?.removePrefix("/channels/")
                ?.decodeURLPart()

    val contextChannelId: String?
        get() = channelId ?: query["channel"]

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

    fun escape(): Boolean {
        val top = overlays.lastOrNull { it.visible && !it.tooltip } ?: return false
        if (top.onEscape?.invoke() != true) top.close()
        return true
    }

    fun navigate(path: String) = platform.navigate(path)

    fun openApp(id: String) {
        target = null
        appDetailId = id
        dialog = "apps"
    }

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
        platform.image(if (url.startsWith("/")) endpoint(config.assetsBaseUrl, url) else url)
            ?: assetClient
                .get(if (url.startsWith("/")) endpoint(config.assetsBaseUrl, url) else url)
                .body()

    fun update(next: Environment) {
        val old = env
        env = next
        toastController.setVisible(next.visible)
        if (old.path.substringBefore('?') != next.path.substringBefore('?')) {
            thread = null
            pane =
                if (
                    channelId != null ||
                        next.path.startsWith("/notifications") ||
                        isHomePath(next.path)
                )
                    "chat"
                else "sidebar"
        }
        if (!old.online && next.online) repo.refresh()
    }

    fun start() {
        activityPubNotifications.ensureLoaded()
        searchPreferences.ensureLoaded()
        pane =
            if (channelId != null || env.path.startsWith("/notifications") || isHomePath(env.path))
                "chat"
            else "sidebar"
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

    suspend fun send(
        channel: String,
        content: String,
        parent: String?,
        commandAppId: String? = null,
        attachmentIds: List<String> = emptyList(),
    ): Boolean {
        val command = Regex("^/([a-zA-Z0-9_-]+)(?:\\s+([\\s\\S]*))?$").matchEntire(content.trim())
        if (command != null && attachmentIds.isEmpty()) {
            try {
                val body = buildJsonObject {
                    put("channel_id", channel)
                    if (commandAppId == null) put("text", command.groupValues[2])
                    else {
                        put("kind", "command")
                        put("name", command.groupValues[1])
                        put("values", obj("text" to command.groupValues[2]))
                    }
                }
                api.request(
                    if (commandAppId == null) "/api/apps/commands/${command.groupValues[1]}"
                    else "/api/apps/${commandAppId.encodeURLParameter()}/invoke",
                    "POST",
                    body,
                )
                return true
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (commandAppId != null || (e as? ApiException)?.status != 404) {
                    toast = t("apps.actionError")
                    return false
                }
            }
        }
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
            if (attachmentIds.isNotEmpty())
                put("attachment_ids", JsonArray(attachmentIds.map(::JsonPrimitive)))
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

    fun configureMembersLayout(wide: Boolean) {
        if (membersWide != wide) {
            membersWide = wide
            membersPanelOpen = false
        }
    }

    private fun membersPath(open: Boolean): String =
        URLBuilder("https://zoo.invalid${env.path}")
            .apply {
                parameters.remove("members")
                if (open) parameters.append("members", "true")
            }
            .buildString()
            .removePrefix("https://zoo.invalid")

    fun removeMembersRequest() {
        if (membersRequested) platform.replace(membersPath(false))
    }

    fun setMembersOpen(open: Boolean, wide: Boolean) {
        configureMembersLayout(wide)
        if (wide) {
            membersPanelOpen = open
            return
        }
        val channel = channelId ?: return
        if (membersRequested == open) return
        if (open) {
            platform.navigate(
                membersPath(true),
                JsonObject(env.historyState + ("zooMembersParent" to JsonPrimitive(channel))),
            )
        } else if (
            env.historyState.text("zooMembersParent") == channel &&
                (env.historyState.text("__TSR_index").toIntOrNull() ?: 0) > 0
        ) {
            platform.action("back")
        } else {
            platform.replace(membersPath(false))
        }
    }

    fun openThread(id: String, wide: Boolean) {
        thread = id
        if (wide) setMembersOpen(false, true)
    }

    fun openMembers(wide: Boolean) {
        configureMembersLayout(wide)
        setMembersOpen(!membersOpen, wide)
        if (membersOpen && wide) thread = null
    }

    private val homeLiveState = lazy { HomeLive(this) }
    val homeLive: HomeLive
        get() = homeLiveState.value

    fun close() {
        if (homeLiveState.isInitialized()) homeLiveState.value.close()
        activityPubNotifications.close()
        toastController.close()
        if (preferenceState.isInitialized()) preferenceState.value.close()
        assetClient.close()
        repo.close()
    }
}

fun String.decodeURLPart(): String = decodeURLQueryComponent()
