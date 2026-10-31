package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
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
    Column(Modifier.fillMaxSize()) {
        Row(
            Modifier.fillMaxWidth().height(64.dp).padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                app.t("notifications.title"),
                Modifier.weight(1f),
                weight = FontWeight.SemiBold,
                size = 18.sp,
            )
            Button(
                app.t("notifications.readAll"),
                app.env.online && notifications.any { !it.flag("is_read") },
            ) {
                app.run { app.api.request("/api/notifications/read-all", "POST", obj()) }
            }
        }
        HorizontalDivider()
        if (groups.isEmpty())
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                Text(app.t("notifications.empty"), muted = true)
            }
        LazyColumn(
            Modifier.weight(1f),
            state = scroll,
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            items(groups, key = { it.first().text("id") }) { rows ->
                NotificationCard(app, rows, users, rows.first().text("id") in visible)
            }
        }
    }
}

@Composable
private fun NotificationCard(app: Session, rows: List<Row>, users: List<Row>, visible: Boolean) {
    val row = rows.first()
    val id = row.text("id")
    val actors = rows.map { it.text("actor_id") }.distinct()
    val user = users.find { it.text("id") == row.text("actor_id") }
    var context by remember(id) { mutableStateOf<Row?>(null) }
    var loading by remember(id) { mutableStateOf(true) }
    LaunchedEffect(id, app.env.visible, app.env.online) {
        if (app.env.visible && app.env.online) {
            try {
                context = app.api.request("/api/notifications/$id/context").objectOrEmpty()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {} finally {
                loading = false
            }
        }
    }
    val read = rows.all { it.flag("is_read") }
    LaunchedEffect(id, read, app.env.visible, app.env.focused, app.env.online, visible) {
        if (!read && app.env.online && app.env.visible && app.env.focused && visible) {
            delay(500)
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
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
        color =
            if (read) MaterialTheme.colorScheme.background else MaterialTheme.colorScheme.surface,
    ) {
        Column(
            Modifier.fillMaxWidth().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Avatar(user)
                Column(Modifier.weight(1f)) {
                    Text(
                        app.t(
                            if (actors.size > 1 && row.text("notification_type") == "reaction")
                                "notifications.reactionGroup"
                            else "notifications.${row.text("notification_type")}",
                            "actor" to (user?.let(::author) ?: row.text("actor_id")),
                            "count" to (actors.size - 1).toString(),
                        )
                    )
                    Text(
                        app.platform.day(row.text("created_at")) +
                            " " +
                            app.platform.time(row.text("created_at")),
                        small = true,
                        muted = true,
                    )
                }
            }
            val message = context?.get("message") as? JsonObject
            if (message != null) RichText(message.text("content").take(560), users)
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
            Row {
                Button(app.t("notifications.openMessage"), message != null) {
                    app.navigate(
                        "/channels/${row.text("channel_id")}?message=${row.text("message_id")}"
                    )
                }
                if (!read)
                    Button(app.t("notifications.read"), app.env.online) {
                        app.run {
                            rows.forEach {
                                app.api.request(
                                    "/api/notifications/${it.text("id")}/read",
                                    "PATCH",
                                    obj(),
                                )
                            }
                        }
                    }
            }
        }
    }
}
