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

    private var visibleIds = emptyList<String>()
    private data class Gap(val anchors: Set<String>, val resume: String?)
    private var gap: Gap? = null
    private var readRevision = 0
    fun viewport(ids: List<String>) {
        val next = ids.take(100)
        if (next != visibleIds && next.any { id -> items.indexOfFirst { it.text("id") == id } >= 100 }) updates.request()
        visibleIds = next
    }
    private var generation = 0
    private var started = false
    private var deferred = false
    private val pendingReads = mutableSetOf<String>()
    private var request: Job? = null
    private var unsubscribe: (() -> Unit)? = null
    private var closed = false
    private var queuedMore: Job? = null
    private val updates = CoalescedRefresh(app.scope, allowed = { !closed && app.env.visible && app.env.online }) { refresh().join() }

    fun ensureLoaded() {
        if (!started) refresh()
    }

    fun refresh(): Job {
        if (closed) return app.scope.launch {}
        request?.takeIf { it.isActive }?.let { deferred = true; return it }
        deferred = false
        started = true
        if (unsubscribe == null) unsubscribe = subscribe { invalidate() }
        val ticket = ++generation
        return app.scope.launch(start = CoroutineStart.LAZY) {
            do {
                deferred = false
                fetch(append = false, ticket = ticket)
            } while (deferred && !closed && app.env.visible && app.env.online)
        }.also { request = it; it.start() }
    }

    fun loadMore() {
        if (closed || nextCursor == null || loading || loadingMore) return
        val running = request?.takeIf { it.isActive }
        if (running != null) {
            if (queuedMore?.isActive != true) queuedMore = app.scope.launch {
                running.join()
                queuedMore = null
                loadMore()
            }
            return
        }
        val ticket = ++generation
        loadingMore = true
        app.scope.launch(start = CoroutineStart.LAZY) {
            fetch(append = true, ticket = ticket)
            if (deferred && !closed) { request = null; refresh() }
        }.also { request = it; it.start() }
    }

    private suspend fun fetch(append: Boolean, ticket: Int) {
        try {
            val readsAtStart = readRevision
            val suffix = if (append) nextCursor?.let { "?before=${it.encodeURLParameter()}" }.orEmpty() else ""
            val response = app.api.request("/api/activitypub/notifications$suffix").objectOrEmpty()
            if (ticket != generation) return
            val incoming = response["items"]?.rows().orEmpty()
            val headIds = incoming.map { it.text("id") }.toSet()
            val check = if (append) emptyList() else visibleIds.filterNot { it in headIds }
            val checked = if (check.isEmpty()) emptyList() else app.api.request(
                "/api/activitypub/notifications/entries", "POST", obj("entries" to JsonArray(check.map(::JsonPrimitive))),
            ).objectOrEmpty()["items"]?.rows().orEmpty()
            if (ticket != generation) return
            val updates = (incoming + checked).associateBy { it.text("id") }
            val retained = items
            val latest = retained.associateBy { it.text("id") }
            val cursor = response.text("next_cursor").takeIf { it.isNotEmpty() }
            items = (if (append) retained + incoming.filterNot { row -> retained.any { it.text("id") == row.text("id") } }
                else incoming + (if (cursor != null) retained.filterNot { it.text("id") in headIds } else emptyList()))
                .filter { it.text("id") !in check || it.text("id") in updates }
                .map { row -> (updates[row.text("id")] ?: row).let { updated ->
                    if (latest[row.text("id")]?.flag("is_read") == true) JsonObject(updated + ("is_read" to JsonPrimitive(true))) else updated
                } }
                .let { rows -> if (rows.any { it.text("created_at").isEmpty() }) rows else rows.sortedWith(compareByDescending<Row> { notificationTimestamp(it.text("created_at")) }.thenByDescending { it.text("id") }) }
                .map { if (it.text("id") in pendingReads) JsonObject(it + ("is_read" to JsonPrimitive(true))) else it }
            if (readsAtStart == readRevision) unreadCount = ((response["unread_count"]?.jsonPrimitive?.intOrNull ?: 0) - incoming.count { !it.flag("is_read") && it.text("id") in pendingReads }).coerceAtLeast(0)
            if (append) {
                val missing = gap
                if (missing != null && incoming.any { it.text("id") in missing.anchors }) {
                    nextCursor = missing.resume
                    gap = null
                } else {
                    nextCursor = cursor
                    if (cursor == null) gap = null
                }
            } else if (cursor == null || retained.isEmpty()) {
                nextCursor = cursor
                gap = null
            } else if (retained.none { it.text("id") in headIds }) {
                if (gap == null) gap = Gap(retained.map { it.text("id") }.toSet(), nextCursor)
                nextCursor = cursor
            } else if (gap == null && nextCursor != null && retained.all { it.text("id") in headIds }) {
                nextCursor = cursor
            }
            error = false
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            if (ticket == generation) error = true
        } finally {
            if (ticket == generation) {
                if (append) loadingMore = false else loading = false
            }
        }
    }

    suspend fun markRead(ids: List<String>? = null) {
        if (ids != null && ids.isEmpty()) return
        val selected = ids?.toSet() ?: items.filterNot { it.flag("is_read") }.map { it.text("id") }.toSet()
        val changed = items.filter { !it.flag("is_read") && it.text("id") in selected }.map { it.text("id") }.toSet()
        readRevision++
        pendingReads.addAll(selected)
        items = items.map { if (it.text("id") in selected) JsonObject(it + ("is_read" to JsonPrimitive(true))) else it }
        val previousCount = unreadCount
        unreadCount = if (ids == null) 0 else (unreadCount - changed.size).coerceAtLeast(0)
        try {
            app.api.request("/api/activitypub/notifications/read", "POST", if (ids == null) obj() else obj("ids" to JsonArray(selected.map(::JsonPrimitive))))
        } catch (e: Exception) {
            readRevision++
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
        if (started && !closed) updates.request()
    }

    fun close() {
        closed = true
        updates.close()
        queuedMore?.cancel()
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
