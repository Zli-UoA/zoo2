package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

@Composable
fun ZooApp(app: Session) {
    if (app.env.path.substringBefore('?') == "/realtime") {
        Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
            Column(
                Modifier.widthIn(max = 896.dp)
                    .fillMaxWidth()
                    .align(Alignment.CenterHorizontally)
                    .padding(32.dp)
            ) {
                Text(app.t("sync.title"), weight = FontWeight.Bold, size = 24.sp)
                Text(app.t("sync.description"), Modifier.padding(top = 8.dp), muted = true)
                Row(
                    Modifier.padding(top = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    DebugBadge("Shape: ${app.config.shapeMode}")
                    DebugBadge("txid: ${app.config.txidMode}")
                    DebugBadge(
                        "${app.t("sync.user")}: ${if (app.env.oidc) app.t("sync.oidc") else app.config.userId}"
                    )
                }
                Button(app.t("sync.back"), modifier = Modifier.padding(top = 16.dp)) {
                    app.navigate("/")
                }
            }
        }
        return
    }
    val channels by app.repo.channels.rows.collectAsState()
    val users by app.repo.users.rows.collectAsState()
    val members by app.repo.members.rows.collectAsState()
    val notifications by app.repo.notifications.rows.collectAsState()
    val loading by app.repo.channels.loading.collectAsState()
    val channel = channels.find { it.text("id") == app.channelId }
    val isNotifications = app.env.path.startsWith("/notifications")
    BoxWithConstraints(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        val width = maxWidth
        val narrow = width < 768.dp
        val wide = width >= 1024.dp
        var wasWide by remember { mutableStateOf(wide) }
        if (wasWide != wide) {
            wasWide = wide
            if (!wide) app.membersOpen = false else if (app.membersOpen) app.thread = null
        }
        if (channel?.flag("is_dm") == true && !channel.text("channel_name").startsWith("dm:group:"))
            app.membersOpen = false
        Row(Modifier.fillMaxSize()) {
            Rail(app, notifications.count { !it.flag("is_read") }, narrow)
            VerticalDivider()
            if (!narrow || (!isNotifications && app.pane == "sidebar")) {
                Sidebar(
                    app,
                    channels,
                    users,
                    members,
                    loading,
                    if (narrow) Modifier.weight(1f) else Modifier.width(287.dp),
                )
                VerticalDivider()
            }
            if (!narrow || isNotifications || app.pane == "chat")
                Column(Modifier.weight(1f).fillMaxHeight()) {
                    when {
                        isNotifications -> Notifications(app, notifications, users)
                        app.env.path.startsWith("/apps-authorize") -> AppConsent(app)
                        app.channelId == null -> Welcome(app)
                        channel == null ->
                            Text(
                                app.t(if (loading) "common.loading" else "channels.notFound"),
                                Modifier.padding(24.dp),
                            )
                        else -> {
                            Header(app, channel, users, members, narrow, wide)
                            HorizontalDivider()
                            Chat(app, channel, users, members, wide, Modifier.weight(1f))
                        }
                    }
                }
            if (
                wide &&
                    app.membersOpen &&
                    channel != null &&
                    (!channel.flag("is_dm") || channel.text("channel_name").startsWith("dm:group:"))
            ) {
                VerticalDivider()
                Members(
                    app,
                    channel,
                    users,
                    members,
                    Modifier.width(if (width >= 1280.dp) 320.dp else 288.dp).fillMaxHeight(),
                )
            }
        }
        if (!app.env.online || app.env.persistenceStatus == "error")
            Surface(
                Modifier.align(Alignment.TopCenter)
                    .padding(top = 8.dp, start = 12.dp, end = 12.dp)
                    .widthIn(max = 576.dp)
                    .semantics { liveRegion = LiveRegionMode.Polite },
                color = MaterialTheme.colorScheme.background,
                shape = RoundedCornerShape(8.dp),
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                shadowElevation = 4.dp,
            ) {
                Row(
                    Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.Top,
                ) {
                    Icon("cloud-off", modifier = Modifier.padding(top = 2.dp).size(16.dp))
                    Column(Modifier.weight(1f, fill = false)) {
                        Text(
                            app.t(
                                if (!app.env.online) "offline.bannerTitle"
                                else "offline.persistenceErrorTitle"
                            ),
                            weight = FontWeight.Medium,
                        )
                        Text(
                            if (!app.env.online) app.t("offline.bannerDescription")
                            else
                                app.env.persistenceError.ifEmpty {
                                    app.t("offline.persistenceErrorDescription")
                                },
                            small = true,
                            muted = true,
                        )
                    }
                    Button(
                        app.t("offline.retry"),
                        modifier =
                            Modifier.border(
                                1.dp,
                                MaterialTheme.colorScheme.outline,
                                RoundedCornerShape(8.dp),
                            ),
                    ) {
                        app.platform.action("retryOffline")
                    }
                }
            }
        app.toast?.let { message ->
            Surface(
                Modifier.align(Alignment.BottomEnd).padding(24.dp).widthIn(max = 360.dp),
                color = MaterialTheme.colorScheme.background,
                shape = RoundedCornerShape(8.dp),
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                shadowElevation = 4.dp,
            ) {
                Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(message, Modifier.weight(1f))
                    IconButton("x", app.t("common.close")) { app.toast = null }
                }
            }
        }
        if (app.membersOpen && !wide && channel != null)
            Panel(app.t("members.title"), { app.membersOpen = false }) {
                Members(app, channel, users, members, Modifier.heightIn(max = 520.dp))
            }
        when (val dialog = app.dialog) {
            "create",
            "browse",
            "dm",
            "rename",
            "moderate",
            "leave" -> ChannelDialog(app, dialog, channel, channels, users, members)
            "message-admin" -> if (channel != null) MessageAdministration(app, channel)
            "apps" -> Apps(app)
            "webhooks" -> if (channel != null) Webhooks(app, channel)
            "theme" ->
                Panel(app.t("theme.title"), { app.dialog = null }, 192.dp) {
                    listOf("light", "dark", "system").forEach { mode ->
                        Button(app.t("theme.$mode")) {
                            app.platform.action("theme:$mode")
                            app.dialog = null
                        }
                    }
                }
            "language" ->
                Panel(app.t("language.title"), { app.dialog = null }, 192.dp) {
                    listOf(
                            "ja" to "日本語",
                            "en" to "English",
                            "zh-Hant" to "繁體中文",
                            "zh-Hans" to "简体中文",
                            "yue" to "廣東話",
                        )
                        .forEach { (key, label) ->
                            Button(label) {
                                app.platform.action("locale:$key")
                                app.dialog = null
                            }
                        }
                }
        }
        AppSurfaces(app)
        val appRoute =
            app.env.path
                .substringBefore('?')
                .takeIf { it.startsWith("/apps/") }
                ?.removePrefix("/apps/")
        if (appRoute != null && app.dialog != "apps")
            Apps(app, appRoute, close = { app.navigate("/") })
        if (app.config.previewUsers.isNotEmpty())
            PreviewUsers(app, Modifier.align(Alignment.BottomEnd).padding(12.dp))
        OverlayHost(app)
    }
}

@Composable
private fun Rail(app: Session, unread: Int, narrow: Boolean) {
    Column(
        Modifier.width(if (narrow) 55.dp else 63.dp)
            .fillMaxHeight()
            .background(MaterialTheme.colorScheme.surface)
            .padding(vertical = 12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Box(
            Modifier.padding(bottom = 4.dp)
                .size(36.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(MaterialTheme.colorScheme.secondary),
            contentAlignment = Alignment.Center,
        ) {
            Icon("paw-print", "Zoo", color = Color.White)
        }
        HorizontalDivider(Modifier.width(24.dp).padding(vertical = 4.dp))
        IconButton(
            "hash",
            app.t("channels.title"),
            selected = app.env.path.substringBefore('?') == "/",
            size = 40.dp,
        ) {
            app.navigate("/")
        }
        Box {
            IconButton(
                "bell",
                app.t("notifications.label"),
                selected = app.env.path.startsWith("/notifications"),
                size = 40.dp,
            ) {
                app.navigate("/notifications")
            }
            if (unread > 0)
                Box(
                    Modifier.align(Alignment.TopEnd)
                        .offset(x = 4.dp, y = (-4).dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(Color(0xffe7000b))
                        .height(16.dp)
                        .widthIn(min = 16.dp)
                        .padding(horizontal = 4.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        if (unread > 99) "99+" else unread.toString(),
                        small = true,
                        size = 10.sp,
                        color = Color.White,
                    )
                }
        }
        IconButton("bot", app.t("apps.title"), size = 28.dp) {
            app.target = null
            app.dialog = "apps"
        }
        if (app.env.development)
            IconButton("activity", app.t("sync.title"), size = 40.dp) { app.navigate("/realtime") }
        Spacer(Modifier.weight(1f))
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(
                Modifier.size(28.dp)
                    .clickable(role = Role.Button) { app.platform.action("react") }
                    .semantics {
                        contentDescription = app.t("experimental.useReact")
                        testTag = "frontend-switch"
                    },
                contentAlignment = Alignment.Center,
            ) {
                Text("R")
            }
            if (app.capabilities.flag("installAvailable"))
                IconButton("download", app.t("pwa.install")) {
                    app.run { app.platform.invoke("install") }
                }
            IconButton(
                if (app.capabilities.flag("pushEnabled")) "bell" else "bell-off",
                app.t(
                    when {
                        !app.capabilities.flag("pushSupported") -> "pwa.unsupported"
                        !app.capabilities.flag("pushConfigured") -> "pwa.unconfigured"
                        app.capabilities.flag("pushEnabled") -> "pwa.disable"
                        else -> "pwa.enable"
                    }
                ),
                enabled =
                    app.capabilities.flag("pushSupported") &&
                        app.capabilities.flag("pushConfigured"),
            ) {
                app.run("push") {
                    app.platform.invoke("push")
                    app.capabilities =
                        json.parseToJsonElement(app.platform.invoke("capabilities")).jsonObject
                }
            }
            IconButton("languages", app.t("language.switch")) { app.dialog = "language" }
            IconButton(if (app.env.dark) "moon" else "sun", app.t("theme.switch")) {
                app.dialog = "theme"
            }
            HorizontalDivider(Modifier.width(24.dp))
            val people by app.repo.users.rows.collectAsState()
            Avatar(people.find { it.text("id") == app.config.userId }, Modifier.size(24.dp))
            if (app.env.oidc)
                IconButton("log-out", app.t("auth.logout"), size = 28.dp) {
                    app.run("logout") { app.platform.invoke("logout") }
                }
        }
    }
}

@Composable
private fun Sidebar(
    app: Session,
    rows: List<Row>,
    users: List<Row>,
    memberships: List<Row>,
    loading: Boolean,
    modifier: Modifier,
) {
    val channels = visibleChannels(rows)
    val joined = channels.filter { isMember(memberships, it.text("id"), app.config.userId) }
    Column(modifier.fillMaxHeight().background(MaterialTheme.colorScheme.surface)) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(app.t("channels.title"), weight = FontWeight.SemiBold)
            Text(
                if (loading) "…" else joined.count { !it.flag("is_dm") }.toString(),
                Modifier.weight(1f).padding(start = 8.dp),
                muted = true,
                small = true,
            )
            IconButton(
                "search",
                app.t("channels.browseTitle"),
                !loading && channels.any { !it.flag("is_dm") },
                size = 28.dp,
            ) {
                app.dialog = "browse"
            }
            IconButton("plus", app.t("channels.createTitle"), size = 28.dp) {
                app.dialog = "create"
            }
        }
        LazyColumn(Modifier.weight(1f).padding(horizontal = 8.dp)) {
            if (loading)
                items(5) {
                    Box(
                        Modifier.fillMaxWidth()
                            .padding(bottom = 4.dp)
                            .height(36.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .background(MaterialTheme.colorScheme.surfaceVariant)
                    )
                }
            if (!loading && joined.none { !it.flag("is_dm") })
                item {
                    Text(app.t("channels.noJoined"), Modifier.padding(8.dp), muted = true)
                    Button(app.t("channels.browseTitle")) { app.dialog = "browse" }
                }
            items(joined.filterNot { it.flag("is_dm") }, key = { it.text("id") }) { channel ->
                ChannelLink(app, channel, channel.text("channel_name"))
            }
            item {
                HorizontalDivider(Modifier.padding(start = 4.dp, end = 4.dp, top = 6.dp))
                Row(
                    Modifier.padding(horizontal = 4.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        app.t("dms.title"),
                        weight = FontWeight.Medium,
                        small = true,
                        muted = true,
                    )
                    Text(
                        joined.count { it.flag("is_dm") }.toString(),
                        Modifier.weight(1f).padding(start = 8.dp),
                        small = true,
                        muted = true,
                    )
                    IconButton("plus", app.t("dms.createTitle"), size = 44.dp) { app.dialog = "dm" }
                }
            }
            if (!loading && joined.none { it.flag("is_dm") })
                item {
                    Column(
                        Modifier.padding(horizontal = 8.dp, vertical = 12.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(app.t("dms.empty"), muted = true)
                        Button(
                            "+ " + app.t("dms.newAction"),
                            modifier =
                                Modifier.heightIn(min = 44.dp)
                                    .background(
                                        MaterialTheme.colorScheme.background,
                                        RoundedCornerShape(8.dp),
                                    )
                                    .border(
                                        1.dp,
                                        MaterialTheme.colorScheme.outline,
                                        RoundedCornerShape(8.dp),
                                    ),
                        ) {
                            app.dialog = "dm"
                        }
                    }
                }
            items(joined.filter { it.flag("is_dm") }, key = { it.text("id") }) { channel ->
                val ids =
                    memberships
                        .filter {
                            it.text("channel_id") == channel.text("id") &&
                                it.text("user_id") != app.config.userId
                        }
                        .map { it.text("user_id") }
                        .toSet()
                ChannelLink(
                    app,
                    channel,
                    users
                        .filter { it.text("id") in ids }
                        .joinToString(", ") { author(it) }
                        .ifEmpty { app.t("dms.untitled") },
                )
            }
        }
        if (app.env.development) {
            HorizontalDivider()
            Column(
                Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    DebugBadge("Shape: ${app.config.shapeMode}")
                    DebugBadge("txid: ${app.config.txidMode}")
                }
                DebugBadge(
                    "${app.t("sync.user")}: ${if (app.env.oidc) app.t("sync.oidc") else app.config.userId}"
                )
            }
        }
    }
}

@Composable
private fun DebugBadge(value: String) {
    Text(
        value,
        Modifier.background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(3.dp))
            .padding(horizontal = 8.dp, vertical = 4.dp),
        small = true,
        muted = true,
    )
}

@Composable
private fun ChannelLink(app: Session, channel: Row, title: String) {
    val selected = app.channelId == channel.text("id")
    Row(
        Modifier.fillMaxWidth()
            .padding(bottom = 2.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(
                if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent
            )
            .clickable {
                app.navigate("/channels/${channel.text("id")}")
                app.pane = "chat"
            }
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (channel.flag("is_dm")) "messages-square"
            else if (channel.flag("is_private")) "lock" else "hash",
            modifier =
                Modifier.size(
                    if (channel.flag("is_dm")) 24.dp
                    else if (channel.flag("is_private")) 14.dp else 16.dp
                ),
            color =
                if (selected) MaterialTheme.colorScheme.onSecondaryContainer.copy(alpha = .6f)
                else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            title,
            Modifier.weight(1f).padding(start = 8.dp),
            muted = !selected,
            maxLines = 1,
            weight =
                if (selected) FontWeight.Medium
                else if (channel.text("id") in app.unread) FontWeight.SemiBold else null,
            color =
                if (selected) MaterialTheme.colorScheme.onSecondaryContainer else Color.Unspecified,
        )
        if (channel.text("id") in app.unread)
            Box(
                Modifier.size(8.dp)
                    .clip(RoundedCornerShape(4.dp))
                    .background(MaterialTheme.colorScheme.secondary)
            )
        if (channel.present("archive_at"))
            Icon("archive", app.t("moderation.archived"), Modifier.size(12.dp))
    }
}

@Composable
private fun Welcome(app: Session) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            Modifier.widthIn(max = 448.dp).padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box(
                Modifier.size(48.dp)
                    .clip(RoundedCornerShape(16.dp))
                    .background(MaterialTheme.colorScheme.secondary),
                contentAlignment = Alignment.Center,
            ) {
                Icon("hash", modifier = Modifier.size(24.dp), color = Color.White)
            }
            Text(app.t("welcome.title"), weight = FontWeight.SemiBold, size = 20.sp)
            Text(app.t("welcome.description"), muted = true)
        }
    }
}

@Composable
private fun Header(
    app: Session,
    channel: Row,
    users: List<Row>,
    memberships: List<Row>,
    narrow: Boolean,
    wide: Boolean,
) {
    val id = channel.text("id")
    val manage = canManage(channel, app.config.userId, app.config.isAdmin)
    Row(
        Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (narrow)
            IconButton("arrow-left", app.t("channels.back"), size = 44.dp) { app.pane = "sidebar" }
        Box(
            Modifier.size(32.dp)
                .clip(RoundedCornerShape(8.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                if (channel.flag("is_dm")) "messages-square"
                else if (channel.flag("is_private")) "lock" else "hash",
                modifier = Modifier.size(16.dp),
            )
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                if (channel.flag("is_dm")) dmTitle(app, channel, users, memberships)
                else channel.text("channel_name"),
                weight = FontWeight.SemiBold,
            )
            Text(
                channel.text("description").ifEmpty { app.t("channels.noDescription") },
                muted = true,
                small = true,
                maxLines = 1,
            )
        }
        if (!channel.flag("is_dm"))
            Row(
                Modifier.height(28.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
                    .clickable(role = Role.Button) { app.openMembers(wide) }
                    .semantics {
                        contentDescription =
                            app.t("members.title") +
                                " " +
                                memberships.count { it.text("channel_id") == id }
                    }
                    .padding(horizontal = 10.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon("users", modifier = Modifier.size(14.dp))
                if (!narrow) Text(app.t("members.title"), small = true)
                Text(
                    memberships.count { it.text("channel_id") == id }.toString(),
                    small = true,
                    muted = true,
                )
            }
        Spacer(Modifier.width(4.dp))
        val actions = mutableListOf<MenuAction>()
        if (channel.flag("is_dm") && channel.text("channel_name").startsWith("dm:group:"))
            actions += MenuAction("users", app.t("members.title")) { app.openMembers(wide) }
        if (manage && !channel.flag("is_dm")) {
            actions +=
                MenuAction("shield-alert", app.t("moderation.messageAdmin")) {
                    app.dialog = "message-admin"
                }
            actions += MenuAction("pencil", app.t("channels.editTitle")) { app.dialog = "rename" }
            actions +=
                MenuAction(
                    "archive",
                    app.t(
                        if (channel.present("archive_at")) "moderation.unarchive"
                        else "moderation.archive"
                    ),
                    app.env.online,
                ) {
                    app.dialog = "moderate"
                }
            actions += MenuAction("webhook", app.t("webhooks.title")) { app.dialog = "webhooks" }
        }
        if (isMember(memberships, id, app.config.userId))
            actions +=
                MenuAction("log-out", app.t("members.leave"), app.env.online) {
                    app.dialog = "leave"
                }
        Menu(app.t("moderation.manageChannel"), actions)
    }
}

@Composable
private fun PreviewUsers(app: Session, modifier: Modifier) {
    var open by remember { mutableStateOf(false) }
    var anchor by remember { mutableStateOf(androidx.compose.ui.geometry.Rect.Zero) }
    Surface(
        modifier,
        color = MaterialTheme.colorScheme.background,
        shape = RoundedCornerShape(6.dp),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
        shadowElevation = 4.dp,
    ) {
        Row(
            Modifier.padding(8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text("Preview ユーザー")
            Row(
                Modifier.onGloballyPositioned { anchor = it.boundsInRoot() }
                    .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(4.dp))
                    .clickable(role = androidx.compose.ui.semantics.Role.Button) { open = true }
                    .semantics { contentDescription = "Preview ユーザー" }
                    .padding(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    app.config.previewUsers
                        .find { it.text("id") == app.config.userId }
                        ?.text("label")
                        .orEmpty()
                )
                Icon("chevron-down", modifier = Modifier.size(12.dp))
            }
        }
    }
    if (open)
        OverlayRegistration("Preview ユーザー", { open = false }, 192.dp, anchor) {
            app.config.previewUsers.forEach { user ->
                Button(user.text("label"), modifier = Modifier.fillMaxWidth()) {
                    open = false
                    app.run { app.platform.invoke("previewUser", user.text("id")) }
                }
            }
        }
}
