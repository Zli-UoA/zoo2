package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
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
    private var request: Job? = null
    private var unsubscribe: (() -> Unit)? = null

    fun ensureLoaded() {
        if (!started) refresh()
    }

    fun refresh(): Job {
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
            val suffix = if (append) "?before=${nextCursor.orEmpty().encodeURLParameter()}" else ""
            val response = app.api.request("/api/activitypub/notifications$suffix").objectOrEmpty()
            if (ticket != generation) return
            val incoming = response["items"]?.rows().orEmpty()
            items =
                if (append)
                    items +
                        incoming.filter { row -> items.none { it.text("id") == row.text("id") } }
                else incoming
            unreadCount = response["unread_count"]?.jsonPrimitive?.intOrNull ?: 0
            nextCursor = response.text("next_cursor").takeIf { it.isNotEmpty() }
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
fun ActivityPubNotifications(
    app: Session,
    state: ActivityPubNotificationsState = app.activityPubNotifications,
) {
    LaunchedEffect(state) { state.ensureLoaded() }
    if (state.items.isEmpty() && !state.error) return
    var pending by remember(state) { mutableStateOf(false) }
    var failed by remember(state) { mutableStateOf(false) }
    fun action(path: String, body: Row) {
        if (pending) return
        pending = true
        failed = false
        app.scope.launch {
            try {
                state.act(path, body)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                failed = true
            } finally {
                pending = false
            }
        }
    }
    Column(
        Modifier.fillMaxWidth().then(webControlModifier("region")).semantics {
            contentDescription = app.t("home.apubNotifications")
        }
    ) {
        Column(
            Modifier.fillMaxWidth().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
            ) {
                Text(
                    app.t("home.apubNotifications"),
                    Modifier.weight(1f).semantics { heading() },
                    weight = FontWeight.SemiBold,
                    size = 14.sp,
                )
                if (state.unreadCount > 0)
                    Button(
                        app.t("home.readAll"),
                        variant = "ghost",
                        size = "sm",
                        enabled = !pending,
                    ) {
                        action("/api/activitypub/notifications/read", obj())
                    }
            }
            if (state.error || failed)
                FlowRow(
                    Modifier.then(webControlModifier("alert")),
                    verticalArrangement = Arrangement.Center,
                ) {
                    Text(
                        app.t("home.controlsError"),
                        Modifier.align(androidx.compose.ui.Alignment.CenterVertically),
                        color = MaterialTheme.colorScheme.error,
                        size = 14.sp,
                    )
                    Button(app.t("home.refresh"), variant = "link") { state.refresh() }
                }
            if (state.items.isNotEmpty())
                Column(
                    Modifier.fillMaxWidth().then(webControlModifier("list")),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    state.items.forEach { item ->
                        val shape = RoundedCornerShape(8.dp)
                        Column(
                            Modifier.fillMaxWidth()
                                .then(webControlModifier("listitem"))
                                .background(
                                    if (!item.flag("is_read"))
                                        (if (MaterialTheme.colorScheme.background == Color.Black)
                                                Color(0xff2e2e2e)
                                            else Color(0xfffffbe8))
                                            .copy(alpha = .5f)
                                    else MaterialTheme.colorScheme.background,
                                    shape,
                                )
                                .border(1.dp, MaterialTheme.colorScheme.outlineVariant, shape)
                                .padding(12.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                                ActivityPubText(
                                    item.text("display_name"),
                                    item["actor_emojis"]?.rows().orEmpty(),
                                    textStyle =
                                        androidx.compose.ui.text.TextStyle(
                                            fontWeight = FontWeight.Bold,
                                            fontSize = 14.sp,
                                            lineHeight = 20.sp,
                                        ),
                                )
                                Text(
                                    app.t("home.notificationKinds.${item.text("kind")}"),
                                    size = 14.sp,
                                )
                            }
                            Text(
                                app.platform.fullTimestamp(item.text("created_at")),
                                size = 12.sp,
                                lineHeight = 16.sp,
                                muted = true,
                            )
                            if (item.text("kind") == "like" && item.text("reaction").isNotEmpty()) {
                                val emoji = item["emoji"] as? JsonObject
                                if (emoji != null) ActivityPubEmoji(emoji, size = 20.dp)
                                else Text(item.text("reaction"), size = 18.sp, lineHeight = 28.sp)
                            }
                            if (item.text("content").isNotEmpty())
                                Text(item.text("content"), size = 14.sp, maxLines = 3)
                            FlowRow(
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                if (item.text("kind") == "follow_request" && item.flag("pending")) {
                                    Button(
                                        app.t("home.approve"),
                                        variant = "default",
                                        size = "sm",
                                        enabled = !pending,
                                    ) {
                                        action(
                                            "/api/activitypub/follows/respond",
                                            obj(
                                                "actor" to item.text("actor_id"),
                                                "activity_id" to item.text("activity_id"),
                                                "accept" to true,
                                            ),
                                        )
                                    }
                                    Button(
                                        app.t("home.reject"),
                                        variant = "outline",
                                        size = "sm",
                                        enabled = !pending,
                                    ) {
                                        action(
                                            "/api/activitypub/follows/respond",
                                            obj(
                                                "actor" to item.text("actor_id"),
                                                "activity_id" to item.text("activity_id"),
                                                "accept" to false,
                                            ),
                                        )
                                    }
                                }
                                if (!item.flag("is_read"))
                                    Button(
                                        app.t("home.markRead"),
                                        variant = "ghost",
                                        size = "sm",
                                        enabled = !pending,
                                    ) {
                                        action(
                                            "/api/activitypub/notifications/read",
                                            obj("id" to item.text("id")),
                                        )
                                    }
                            }
                        }
                    }
                }
            if (state.nextCursor != null)
                Button(
                    app.t(if (state.loadingMore) "home.loading" else "home.moreNotifications"),
                    variant = "outline",
                    enabled = !pending && !state.loadingMore,
                ) {
                    state.loadMore()
                }
        }
        HorizontalDivider()
    }
}
