package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.*
import kotlinx.serialization.json.*

/** Story entry points use the same screen and feature functions as ZooApp. */
@Composable
fun ShellStory(app: Session, options: Row) {
    val fixture = options.nested("fixture")
    val title = fixture.text("title")
    val state = fixture.text("state")
    val dialogVisible =
        title == "Features/Channels/Edit" || !fixture.containsKey("open") || fixture.flag("open")
    fun callback(name: String, args: List<JsonElement> = emptyList()) {
        if (fixture.array("callbacks").any { it.jsonPrimitive.content == name })
            app.run {
                app.platform.invoke(
                    "storyCallback",
                    obj("name" to name, "args" to JsonArray(args)).toString(),
                )
            }
    }
    fun rows(key: String) = fixture.array(key).map { it.jsonObject }
    LaunchedEffect(title, state) {
        app.admins =
            app.api
                .request("/api/users/admins")
                .jsonObject
                .array("user_ids")
                .map { it.jsonPrimitive.content }
                .toSet()
        if (title == "Features/Channels/Sidebar" || title == "Pages/Shell") {
            app.unread =
                app.api
                    .request("/api/channel-read-states")
                    .rows()
                    .filter { it.flag("has_unread") }
                    .map { it.text("channel_id") }
                    .toSet()
        }
    }
    val channels = rows("channels")
    val users = rows("users")
    val syncedMembers by app.repo.members.rows.collectAsState()
    val members = if (fixture.containsKey("memberships")) rows("memberships") else syncedMembers
    val channel =
        fixture.nested("channel").takeIf { it.isNotEmpty() }
            ?: channels.find {
                it.text("id") == fixture.text("channelId").ifEmpty { "channel-general" }
            }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val viewportWidth = options.text("viewportWidth").toFloatOrNull()?.dp ?: maxWidth
        val narrow = viewportWidth < 768.dp
        when (title) {
            "Pages/Shell" -> {
                LaunchedEffect(state) { app.membersOpen = fixture.flag("membersOpen") }
                ZooApp(app)
            }
            "Pages/Welcome" -> Welcome(app)
            "Pages/Skeleton" -> ShellSkeleton()
            "Features/Navigation/Rail" ->
                Rail(app, rows("notifications").count { !it.flag("is_read") }, narrow)
            "Features/Channels/Sidebar" ->
                Row(Modifier.fillMaxSize()) {
                    Sidebar(
                        app,
                        channels,
                        users,
                        members,
                        fixture.flag("loading"),
                        if (narrow) Modifier.weight(1f) else Modifier.width(287.dp),
                        fixture.flag("error"),
                        narrow,
                    )
                    VerticalDivider()
                }
            "Features/DMs/ListItem" ->
                fixture.nested("channel").let { direct ->
                    DmItem(
                        app,
                        direct,
                        fixture.text("label"),
                        fixture.flag("active"),
                        fixture.flag("unread"),
                        narrow,
                        onOpen = { callback("onOpen") },
                    )
                }
            "Features/DMs/Sidebar" ->
                DmSidebar(
                    app,
                    channels,
                    users,
                    members,
                    fixture.flag("loading"),
                    fixture.flag("error"),
                    fixture.flag("usersError"),
                    fixture.text("activeChannelId"),
                    fixture.array("unreadIds").map { it.jsonPrimitive.content }.toSet(),
                    narrow,
                    onCreate = { callback("onCreate") },
                    onOpen = { callback("onOpen") },
                    onReload = { callback("onReload") },
                )
            "Features/Channels/Header" ->
                if (channel != null) {
                    Column {
                        Header(
                            app,
                            channel,
                            users,
                            members,
                            narrow,
                            viewportWidth >= 1024.dp,
                            fixture,
                        )
                        HorizontalDivider()
                    }
                    AppDialogs(app, channel, channels, users, members)
                }
            "Pages/Channel" ->
                Column(Modifier.fillMaxSize()) {
                    if (channel == null) Text(app.t("channels.notFound"), Modifier.padding(24.dp))
                    else {
                        Header(app, channel, users, members, narrow, viewportWidth >= 1024.dp)
                        HorizontalDivider()
                        Chat(
                            app,
                            channel,
                            users,
                            members,
                            viewportWidth >= 1024.dp,
                            Modifier.weight(1f),
                        )
                    }
                }
            "Features/Channels/Members" ->
                if (channel != null)
                    Members(app, channel, users, members, Modifier.fillMaxSize(), false)
            "Pages/ChannelMembers" ->
                if (channel != null)
                    Members(
                        app,
                        channel,
                        users,
                        members,
                        Modifier.fillMaxSize(),
                        asScreen = true,
                        onBack = { callback("onBack") },
                    )
            "Pages/BubbleChat" -> BubbleChat(app, users)
            "Features/Channels/Create",
            "Features/Channels/Edit",
            "Features/Channels/Discovery",
            "Features/DMs/Create" -> {
                val kind =
                    when (title) {
                        "Features/Channels/Create" -> "create"
                        "Features/Channels/Edit" -> "rename"
                        "Features/Channels/Discovery" -> "browse"
                        else -> "dm"
                    }
                val initial = JsonObject(emptyMap())
                ChannelDialog(
                    app,
                    kind,
                    channel,
                    channels,
                    users,
                    members,
                    JsonObject(initial + fixture.filterKeys { it == "joinedIds" }),
                    open = dialogVisible,
                    onOpenChange = { open -> callback("onOpenChange", listOf(JsonPrimitive(open))) },
                )
            }
            else -> error("No shell renderer for $title / $state")
        }
        if (title != "Pages/Shell" && title != "Features/Channels/Header" && app.dialog != null)
            AppDialogs(app, channel, channels, users, members)
    }
}

@Composable
internal fun ChannelMembersScreen(
    app: Session,
    channel: Row,
    users: List<Row>,
    members: List<Row>,
) {
    Members(app, channel, users, members, Modifier.fillMaxSize(), asScreen = true)
}
