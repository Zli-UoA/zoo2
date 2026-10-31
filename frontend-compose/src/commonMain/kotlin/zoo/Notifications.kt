package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

fun notificationGroups(rows: List<Row>): List<List<Row>> =
    rows
        .sortedWith(
            compareByDescending<Row> { timestampMillis(it.text("created_at")) }
                .thenByDescending { it.text("id") }
        )
        .groupBy {
            if (it.text("notification_type") == "reaction")
                "reaction:${it.text("user_id")}:${it.text("channel_id")}:${it.text("message_id")}"
            else "notification:${it.text("id")}"
        }
        .values
        .toList()

@Composable
fun Notifications(app: Session, notifications: List<Row>, users: List<Row>) {
    val groups = notificationGroups(notifications)
    val initialLoading by app.repo.notifications.loading.collectAsState()
    val loadError by app.repo.notifications.error.collectAsState()
    val contextLoading = remember { mutableStateMapOf<String, Boolean>() }

    val scroll = rememberLazyListState()
    val visible by remember {
        derivedStateOf {
            val layout = scroll.layoutInfo
            layout.visibleItemsInfo
                .filter { item ->
                    val overlap =
                        minOf(item.offset + item.size, layout.viewportEndOffset) -
                            maxOf(item.offset, layout.viewportStartOffset)
                    overlap > 0 && overlap.toFloat() / item.size.coerceAtLeast(1) >= .1f
                }
                .map { it.key.toString() }
                .toSet()
        }
    }
    var hasLoaded by remember { mutableStateOf(false) }
    val loading = initialLoading || visible.any { contextLoading[it] != false }
    LaunchedEffect(loading, visible) {
        if (!loading && (groups.isEmpty() || visible.isNotEmpty())) hasLoaded = true
    }
    val initialMask = loading && !hasLoaded
    Column(Modifier.fillMaxSize()) {
        Row(
            Modifier.fillMaxWidth().height(64.dp).padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                Modifier.size(32.dp)
                    .background(MaterialTheme.colorScheme.primary, RoundedCornerShape(8.dp)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    "bell",
                    modifier = Modifier.size(16.dp),
                    color = MaterialTheme.colorScheme.onPrimary,
                )
            }
            Spacer(Modifier.width(12.dp))
            Text(
                app.t("notifications.title"),
                Modifier.semantics { heading() },
                weight = FontWeight.SemiBold,
            )
        }
        HorizontalDivider()
        if (loadError != null) {
            Text(app.t("notifications.loadError"), Modifier.padding(16.dp))
            Button(app.t("offline.retry")) { app.repo.notifications.refresh() }
        }
        Box(Modifier.weight(1f)) {
            LazyColumn(
                Modifier.fillMaxSize().alpha(if (initialMask) 0f else 1f).semantics {
                    if (initialMask) invisibleToUser()
                },
                state = scroll,
                contentPadding = PaddingValues(0.dp),
            ) {
                items(groups, key = { it.first().text("id") }) { rows ->
                    NotificationCard(
                        app,
                        rows,
                        users,
                        rows.first().text("id") in visible,
                        !loading,
                    ) {
                        contextLoading[rows.first().text("id")] = it
                    }
                }
            }
            if (groups.isEmpty() && !initialLoading)
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(app.t("notifications.empty"), muted = true)
                }
            if (loading)
                Box(
                    Modifier.fillMaxWidth()
                        .align(if (initialMask) Alignment.Center else Alignment.BottomCenter)
                        .background(MaterialTheme.colorScheme.background)
                        .padding(16.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                        Text(
                            app.t("common.loading"),
                            Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                            muted = true,
                        )
                    }
                }
        }
    }
}

@Composable
private fun NotificationCard(
    app: Session,
    rows: List<Row>,
    users: List<Row>,
    visible: Boolean,
    readEnabled: Boolean,
    onLoading: (Boolean) -> Unit,
) {
    val row = rows.first()
    val id = row.text("id")
    val actors = rows.map { it.text("actor_id") }.distinct()
    val user = users.find { it.text("id") == row.text("actor_id") }
    var context by remember(id) { mutableStateOf<Row?>(null) }
    var loading by remember(id) { mutableStateOf(true) }
    var wasVisible by remember(id) { mutableStateOf(false) }
    LaunchedEffect(visible) { if (visible) wasVisible = true }
    LaunchedEffect(id, wasVisible, app.env.online) {
        if (wasVisible && app.env.online) {
            loading = true
            onLoading(true)
            try {
                context = app.api.request("/api/notifications/$id/context").objectOrEmpty()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {} finally {
                loading = false
                onLoading(false)
            }
        }
    }
    val read = rows.all { it.flag("is_read") }
    val message = context?.get("message") as? JsonObject
    val channels by app.repo.channels.rows.collectAsState()
    val memberships by app.repo.members.rows.collectAsState()
    val channel = channels.find { it.text("id") == row.text("channel_id") }
    val destination = "/channels/${row.text("channel_id")}?message=${row.text("message_id")}"
    val actor = user?.let(::author) ?: row.text("actor_id")
    var attempted by remember(id) { mutableStateOf(false) }
    LaunchedEffect(
        id,
        read,
        app.env.visible,
        app.env.focused,
        app.env.online,
        visible,
        readEnabled,
    ) {
        if (
            !read &&
                !attempted &&
                readEnabled &&
                app.env.online &&
                app.env.visible &&
                app.env.focused &&
                visible
        ) {
            delay(500)
            attempted = true
            rows
                .filterNot { it.flag("is_read") }
                .forEach {
                    try {
                        app.api.request("/api/notifications/${it.text("id")}/read", "PATCH", obj())
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {}
                }
        }
    }
    Surface(
        color =
            if (read) MaterialTheme.colorScheme.background else MaterialTheme.colorScheme.surface
    ) {
        Column(
            Modifier.fillMaxWidth().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Avatar(user, Modifier.size(36.dp))
                Row(
                    Modifier.weight(1f),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        if (actors.size > 1)
                            app.t(
                                "notifications.actors",
                                "actor" to actor,
                                "count" to (actors.size - 1).toString(),
                            )
                        else actor,
                        Modifier.weight(1f),
                        weight = FontWeight.SemiBold,
                        maxLines = 1,
                    )
                    Text(
                        app.platform.day(row.text("created_at")) +
                            " " +
                            app.platform.time(row.text("created_at")),
                        small = true,
                        muted = true,
                    )
                    if (!read) Text(app.t("notifications.unreadStatus"), small = true)
                    else
                        Icon(
                            "check-check",
                            app.t("notifications.readStatus"),
                            modifier = Modifier.size(16.dp),
                        )
                }
            }
            if (message != null && channel != null)
                RichText(quotePreview(message.text("content")), users)
            else
                Text(
                    app.t(if (loading) "common.loading" else "notifications.messageUnavailable"),
                    small = true,
                    muted = true,
                )
            if (row.text("notification_type") == "reaction")
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    context
                        ?.get("reactions")
                        ?.rows()
                        ?.filter { it.text("user_id") in actors }
                        ?.groupBy { reactionKey(it.text("emoji")) }
                        ?.forEach { (emoji, reactions) ->
                            ReactionEmoji(emoji)
                            Text(
                                reactions.map { it.text("user_id") }.distinct().size.toString(),
                                small = true,
                            )
                        }
                }
            val label =
                if (channel == null) app.t("notifications.unavailable")
                else if (channel.flag("is_dm")) dmTitle(app, channel, users, memberships)
                else "#${channel.text("channel_name")}"
            Button(label, channel != null && message != null) { app.navigate(destination) }
        }
    }
    HorizontalDivider()
}

fun quotePreview(value: String): String {
    var i = 0
    var count = 0
    while (i < value.length && count < 280) {
        val ch = value[i++]
        if (ch.isHighSurrogate() && i < value.length && value[i].isLowSurrogate()) i++
        count++
    }
    return value.take(i) + if (i < value.length) "…" else ""
}
