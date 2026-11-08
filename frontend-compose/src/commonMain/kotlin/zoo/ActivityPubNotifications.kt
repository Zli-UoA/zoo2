package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

/** The notification feed is shared by the rail count and its screen. */
class ActivityPubNotificationsState(
    private val app: Session,
    private val subscribe: (() -> Unit) -> (() -> Unit) = { app.homeLive.subscribe(it) },
) {
    var items by mutableStateOf<List<Row>>(emptyList())
        private set

    var unreadCount by mutableStateOf(0)
        private set

    var loading by mutableStateOf(true)
        private set

    var loadingMore by mutableStateOf(false)
        private set

    var error by mutableStateOf(false)
        private set

    var nextCursor by mutableStateOf<String?>(null)
        private set

    private var generation = 0
    private var started = false
    private var deferred = false
    private val pendingReads = mutableSetOf<String>()
    private var request: Job? = null
    private var unsubscribe: (() -> Unit)? = null

    fun ensureLoaded() {
        if (!started) refresh()
    }

    fun refresh(): Job {
        if (loadingMore) { deferred = true; return request ?: app.scope.launch {} }
        deferred = false
        started = true
        if (unsubscribe == null) unsubscribe = subscribe { invalidate() }
        val ticket = ++generation
        request?.cancel()
        loadingMore = false
        return app.scope.launch { fetch(append = false, ticket = ticket) }.also { request = it }
    }

    fun loadMore() {
        if (nextCursor == null || loading || loadingMore) return
        val ticket = ++generation
        request?.cancel()
        loadingMore = true
        request = app.scope.launch { fetch(append = true, ticket = ticket) }
    }

    private suspend fun fetch(append: Boolean, ticket: Int) {
        try {
            val retained = items
            val tail = retained.lastOrNull()
            val incoming = mutableListOf<Row>()
            val seen = mutableSetOf<String>()
            var cursor = if (append) nextCursor else null
            var response: Row
            do {
                val suffix = cursor?.let { "?before=${it.encodeURLParameter()}" }.orEmpty()
                response = app.api.request("/api/activitypub/notifications$suffix").objectOrEmpty()
                if (ticket != generation) return
                incoming.addAll(response["items"]?.rows().orEmpty().filter { row -> incoming.none { it.text("id") == row.text("id") } })
                cursor = response.text("next_cursor").takeIf { it.isNotEmpty() }
                val last = incoming.lastOrNull()
                val reached = tail == null || incoming.any { it.text("id") == tail.text("id") } || last != null &&
                    (notificationTimestamp(last.text("created_at")) < notificationTimestamp(tail.text("created_at")) ||
                    notificationTimestamp(last.text("created_at")) == notificationTimestamp(tail.text("created_at")) && last.text("id") <= tail.text("id"))
                if (append || cursor == null || !seen.add(cursor) || reached && incoming.size >= retained.size) break
            } while (true)
            items = (if (append) items + incoming.filter { row -> items.none { it.text("id") == row.text("id") } } else incoming)
                .map { if (it.text("id") in pendingReads) JsonObject(it + ("is_read" to JsonPrimitive(true))) else it }
            unreadCount = response["unread_count"]?.jsonPrimitive?.intOrNull ?: 0
            nextCursor = cursor
            error = false
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            if (ticket == generation) error = true
        } finally {
            if (ticket == generation) {
                if (append) { loadingMore = false; if (deferred) refresh() } else loading = false
            }
        }
    }

    suspend fun markRead(ids: List<String>? = null) {
        if (ids != null && ids.isEmpty()) return
        val selected = ids?.toSet() ?: items.filterNot { it.flag("is_read") }.map { it.text("id") }.toSet()
        val changed = items.filter { !it.flag("is_read") && it.text("id") in selected }.map { it.text("id") }.toSet()
        pendingReads.addAll(selected)
        items = items.map { if (it.text("id") in selected) JsonObject(it + ("is_read" to JsonPrimitive(true))) else it }
        val previousCount = unreadCount
        unreadCount = if (ids == null) 0 else (unreadCount - changed.size).coerceAtLeast(0)
        try {
            app.api.request("/api/activitypub/notifications/read", "POST", if (ids == null) obj() else obj("ids" to JsonArray(selected.map(::JsonPrimitive))))
        } catch (e: Exception) {
            items = items.map { if (it.text("id") in changed) JsonObject(it + ("is_read" to JsonPrimitive(false))) else it }
            unreadCount = previousCount
            throw e
        } finally { pendingReads.removeAll(selected) }
    }

    suspend fun act(path: String, body: Row) {
        app.api.request(path, "POST", body)
        refresh().join()
    }

    fun invalidate() {
        if (started) refresh()
    }

    fun close() {
        generation++
        request?.cancel()
        unsubscribe?.invoke()
        unsubscribe = null
    }
}

@Composable
fun ActivityPubNotificationCard(
    app: Session, item: Row, state: ActivityPubNotificationsState,
    visible: Boolean, readEnabled: Boolean, onRead: (List<String>) -> Unit,
) {
    val id = item.text("id")
    var attempted by remember(id) { mutableStateOf(false) }
    LaunchedEffect(id, item.flag("is_read"), visible, readEnabled, app.env.focused, app.env.visible, app.env.online) {
        if (!attempted && !item.flag("is_read") && visible && readEnabled && app.env.focused && app.env.visible && app.env.online) {
            delay(500); attempted = true; onRead(listOf("activitypub:$id"))
        }
    }
    var pending by remember(id) { mutableStateOf(false) }
    var failed by remember(id) { mutableStateOf(false) }
    fun respond(accept: Boolean) {
        if (pending) return
        pending = true; failed = false
        app.scope.launch {
            try { state.act("/api/activitypub/follows/respond", obj("actor" to item.text("actor_id"), "activity_id" to item.text("activity_id"), "accept" to accept)) }
            catch (e: CancellationException) { throw e }
            catch (_: Exception) { failed = true }
            finally { pending = false }
        }
    }
    val profile = "/home/users/${item.text("actor_id").removePrefix("local:").encodeURLParameter()}"
    val destination = item.text("post_id").takeIf { it.isNotEmpty() }?.let { "/home/posts/${it.encodeURLParameter()}" }
    val label = "${item.text("display_name")} ${app.t("home.notificationKinds.${item.text("kind")}")}"
    BoxWithConstraints {
        val mobile = maxWidth < 640.dp
        val avatarSize = if (mobile) 40.dp else 36.dp
        Column(
            Modifier.notificationSurface(item.flag("is_read"))
                .then(webControlModifier("listitem", attributes = mapOf("aria-label" to label)))
                .then(if (destination != null) Modifier.nativeNavigation(destination, role = null) else Modifier)
                .padding(12.dp),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                NotificationAvatar(
                    size = avatarSize,
                    avatar = { size -> NativeLink(profile) { ActivityPubAvatar(item["avatar"] as? Row, item.text("display_name"), size = size) } },
                ) {
                    if (item.text("kind") == "like") {
                        val emoji = item["emoji"] as? Row
                        val reaction = emoji?.text("name") ?: item.text("reaction").ifEmpty { "❤️" }
                        Box(Modifier.then(webControlModifier("img", attributes = mapOf("aria-label" to reaction, "data-notification-reaction" to "")))) {
                            if (emoji != null) ActivityPubEmoji(emoji, size = 16.dp) else Text(reaction, size = 16.sp)
                        }
                    } else Icon(when(item.text("kind")) { "reply" -> "message-circle"; "repost" -> "repeat-2"; "follow_request" -> "user-plus"; "follow_accepted" -> "user-check"; else -> "at-sign" }, modifier = Modifier.size(12.dp))
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(if (mobile) 8.dp else 4.dp)) {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                        NativeLink(profile, Modifier.weight(1f)) { ActivityPubText(item.text("display_name"), item.homeRows("actor_emojis"), textStyle = androidx.compose.ui.text.TextStyle(fontSize = 14.sp, fontWeight = FontWeight.SemiBold)) }
                        NotificationTimestamp(app, item.text("created_at"))
                        NotificationReadStatus(app, item.flag("is_read"))
                    }
                    if (item.text("content").isNotEmpty()) ActivityPubContent(item.text("content"), item.homeRows("content_emojis"), maxLines = 3)
                    if (destination != null) NativeLink(destination, attributes = mapOf("aria-label" to "${app.t("home.title")} · ${app.t("activitypubSettings.post")} — $label")) {
                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) { Icon("home", modifier = Modifier.size(16.dp)); Text(app.t("home.title"), size = 12.sp, muted = true) }
                    }
                    if (item.text("kind") == "follow_request" && item.flag("pending")) FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(app.t("home.approve"), size = "sm", enabled = !pending) { respond(true) }
                        Button(app.t("home.reject"), variant = "outline", size = "sm", enabled = !pending) { respond(false) }
                    }
                    if (failed) Text(app.t("home.controlsError"), Modifier.then(webControlModifier("alert")), color = MaterialTheme.colorScheme.error, size = 14.sp)
                }
            }
        }
    }
}
