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
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

@Composable
fun ZooApp(app: Session) {
    if (app.env.path.substringBefore('?') == "/realtime") {
        Column(
            Modifier.fillMaxSize()
                .focusInputModality()
                .background(MaterialTheme.colorScheme.background)
        ) {
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
    val channelRows by app.repo.channels.rows.collectAsState()
    val channels = visibleChannels(channelRows)
    val users by app.repo.users.rows.collectAsState()
    val members by app.repo.members.rows.collectAsState()
    val notifications by app.repo.notifications.rows.collectAsState()
    val loading by app.repo.channels.loading.collectAsState()
    val channel = channels.find { it.text("id") == app.channelId }
    val isNotifications = app.env.path.startsWith("/notifications")
    val isSettings = app.env.path.startsWith("/settings")
    val isBubble = app.env.path.startsWith("/bubble")
    val fullPage =
        isNotifications || isSettings || isBubble || app.env.path.startsWith("/apps-authorize")
    BoxWithConstraints(
        Modifier.fillMaxSize().focusInputModality().background(MaterialTheme.colorScheme.background)
    ) {
        val width = maxWidth
        val narrow = width < 768.dp
        val wide = width >= 1024.dp
        app.configureMembersLayout(wide)
        LaunchedEffect(wide, app.env.path) { if (wide) app.removeMembersRequest() }
        if (wide && channel?.flag("is_dm") == true) app.membersOpen = false
        val membersChannel = channel?.takeUnless { it.flag("is_dm") }
        val showMembersScreen = app.membersOpen && !wide && membersChannel != null
        Row(Modifier.fillMaxSize()) {
            if (!isSettings && !showMembersScreen) {
                Rail(app, notifications.count { !it.flag("is_read") }, narrow)
                VerticalDivider()
            }
            if (!fullPage && (!narrow || app.pane == "sidebar") && !showMembersScreen) {
                Sidebar(
                    app,
                    channels,
                    users,
                    members,
                    loading,
                    if (narrow) Modifier.weight(1f) else Modifier.width(287.dp),
                    narrow = narrow,
                )
                VerticalDivider()
            }
            if (!narrow || fullPage || app.pane == "chat")
                Column(Modifier.weight(1f).fillMaxHeight().then(webControlModifier("main"))) {
                    when {
                        showMembersScreen ->
                            ChannelMembersScreen(app, membersChannel!!, users, members)
                        isSettings -> ZooSettings(app, users)
                        isBubble -> BubbleChat(app, users)
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
            if (wide && app.membersOpen && membersChannel != null) {
                VerticalDivider()
                Column(Modifier.width(if (width >= 1280.dp) 320.dp else 288.dp).fillMaxHeight()) {
                    Members(app, membersChannel, users, members, Modifier.fillMaxSize())
                }
            }
        }
        OfflineBanner(
            app,
            obj(
                "isOnline" to app.env.online,
                "persistenceStatus" to app.env.persistenceStatus,
                "persistenceError" to app.env.persistenceError,
            ),
        )
        AppDialogs(app, channel, channels, users, members)
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
    }
}

@Composable
internal fun AppDialogs(
    app: Session,
    channel: Row?,
    channels: List<Row>,
    users: List<Row>,
    members: List<Row>,
) {
    when (val dialog = app.dialog) {
        "create",
        "browse",
        "dm",
        "rename",
        "moderate",
        "leave" -> ChannelDialog(app, dialog, channel, channels, users, members)
        "channel-settings" ->
            if (channel != null)
                Panel(
                    app.t("settings.channelTitle"),
                    { app.dialog = null },
                    512.dp,
                    description = channel.text("channel_name"),
                ) {
                    Column(
                        Modifier.fillMaxWidth()
                            .border(
                                1.dp,
                                MaterialTheme.colorScheme.outline,
                                RoundedCornerShape(10.dp),
                            )
                    ) {
                        SettingRow(
                            app.t("channels.editTitle"),
                            channel.text("description").trim().ifEmpty {
                                app.t("channels.noDescription")
                            },
                        ) {
                            Button(
                                app.t("messages.edit"),
                                !channel.present("archive_at"),
                                variant = "outline",
                                icon = "pencil",
                            ) {
                                app.dialog = "rename"
                            }
                        }
                        HorizontalDivider()
                        SettingRow(app.t("webhooks.title")) {
                            Button(app.t("common.open"), variant = "outline", icon = "webhook") {
                                app.dialog = "webhooks"
                            }
                        }
                    }
                    SettingsSection("") {
                        Text(
                            app.t(
                                if (channel.present("archive_at")) "moderation.unarchive"
                                else "moderation.archive"
                            ),
                            weight = FontWeight.Medium,
                        )
                        Text(
                            app.t(
                                if (channel.present("archive_at")) "moderation.unarchiveDescription"
                                else "moderation.archiveDescription"
                            ),
                            small = true,
                            muted = true,
                        )
                        Button(
                            app.t(
                                if (channel.present("archive_at")) "moderation.unarchive"
                                else "moderation.archive"
                            ),
                            variant =
                                if (channel.present("archive_at")) "outline" else "destructive",
                            icon = "archive",
                        ) {
                            app.dialog = "moderate"
                        }
                    }
                }
        "message-admin" -> if (channel != null) MessageAdministration(app, channel)
        "apps" ->
            Apps(
                app,
                initial = app.appDetailId,
                close = {
                    app.dialog = null
                    app.appDetailId = null
                    app.target = null
                },
            )
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
}

@Composable
internal fun Rail(app: Session, unread: Int, narrow: Boolean) {
    Column(
        Modifier.width(if (narrow) 55.dp else 63.dp)
            .fillMaxHeight()
            .then(webControlModifier("navigation"))
            .semantics { contentDescription = app.t("nav.main") }
            .background(if (app.env.dark) Color(0xff141414) else Color(0xfffafaf8))
            .padding(bottom = 12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Column {
            Box(Modifier.fillMaxWidth().height(60.dp), contentAlignment = Alignment.Center) {
                Box(
                    Modifier.size(36.dp)
                        .clip(RoundedCornerShape(14.dp))
                        .background(MaterialTheme.colorScheme.secondary),
                    contentAlignment = Alignment.Center,
                ) {
                    BrandLogo(Modifier.fillMaxSize())
                }
            }
            HorizontalDivider()
            Spacer(Modifier.height(4.dp))
        }
        RailLink(app, "/", "hash", app.t("channels.title"))
        NotificationsButton(app, unread)
        RailLink(app, "/bubble", "message-circle-more", app.t("bubbleChat.title"))
        if (app.env.development) RailLink(app, "/realtime", "activity", app.t("sync.title"))
        Spacer(Modifier.weight(1f))
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            FrontendSwitch(app)
            RailLink(
                app,
                "/settings" + (app.channelId?.let { "?channel=$it" } ?: ""),
                "settings",
                app.t("settings.zooTitle"),
            )
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
internal fun Sidebar(
    app: Session,
    rows: List<Row>,
    users: List<Row>,
    memberships: List<Row>,
    loading: Boolean,
    modifier: Modifier,
    error: Boolean = false,
    narrow: Boolean = false,
) {
    val channels = rows.filterNot { it.present("deleted_at") }
    val joined = channels.filter { isMember(memberships, it.text("id"), app.config.userId) }
    BoxWithConstraints(
        modifier
            .fillMaxHeight()
            .then(webControlModifier("complementary"))
            .semantics { contentDescription = app.t("channels.title") }
            .background(if (app.env.dark) Color(0xff141414) else Color(0xfffafaf8))
    ) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(vertical = 4.dp)
        ) {
            CollapsibleSection(
                app.t("channels.title"),
                icon = "hash",
                actions = {
                    IconButton(
                        "search",
                        app.t("channels.browseTitle"),
                        !loading && channels.any { !it.flag("is_dm") },
                        size = 28.dp,
                        color = MaterialTheme.colorScheme.onSurface,
                    ) {
                        app.dialog = "browse"
                    }
                    IconButton(
                        "plus",
                        app.t("channels.createTitle"),
                        size = 28.dp,
                        color = MaterialTheme.colorScheme.onSurface,
                    ) {
                        app.dialog = "create"
                    }
                },
            ) {
                Column(
                    Modifier.fillMaxWidth().padding(start = 8.dp, end = 8.dp, bottom = 8.dp),
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    when {
                        loading ->
                            repeat(5) {
                                Box(
                                    Modifier.fillMaxWidth()
                                        .height(if (narrow) 36.dp else 32.dp)
                                        .background(
                                            MaterialTheme.colorScheme.surfaceVariant,
                                            RoundedCornerShape(8.dp),
                                        )
                                )
                            }
                        error ->
                            Text(
                                app.t("channels.loadError"),
                                Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                                small = true,
                                color = MaterialTheme.colorScheme.error,
                            )
                        channels.none { !it.flag("is_dm") } ->
                            Column(
                                Modifier.padding(8.dp),
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Text(app.t("channels.empty"), muted = true)
                                Button(
                                    app.t("channels.createAction"),
                                    variant = "outline",
                                    size = "sm",
                                    icon = "plus",
                                ) {
                                    app.dialog = "create"
                                }
                            }
                        joined.none { !it.flag("is_dm") } ->
                            Column(
                                Modifier.padding(8.dp),
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Text(app.t("channels.noJoined"), muted = true)
                                Button(
                                    app.t("channels.browseTitle"),
                                    variant = "outline",
                                    size = "sm",
                                    icon = "search",
                                ) {
                                    app.dialog = "browse"
                                }
                            }
                        else ->
                            joined
                                .filterNot { it.flag("is_dm") }
                                .forEach { channel ->
                                    ChannelLink(app, channel, channel.text("channel_name"))
                                }
                    }
                }
            }
            DmSidebar(
                app,
                joined.filter { it.flag("is_dm") },
                users,
                memberships,
                loading,
                error,
                false,
                app.channelId.orEmpty(),
                app.unread,
                narrow,
            )
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
internal fun ChannelLink(app: Session, channel: Row, title: String) {
    val selected = app.channelId == channel.text("id")
    val windowWidth = LocalWindowInfo.current.containerSize.width
    val narrow = with(LocalDensity.current) { windowWidth.toDp() < 768.dp }
    Row(
        Modifier.fillMaxWidth()
            .height(if (narrow) 36.dp else 32.dp)
            .then(
                webControlModifier(
                    "link",
                    attributes = if (selected) mapOf("aria-current" to "page") else emptyMap(),
                )
            )
            .clip(RoundedCornerShape(8.dp))
            .background(
                if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent
            )
            .clickable {
                app.navigate("/channels/${channel.text("id")}")
                app.pane = "chat"
            }
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (channel.flag("is_dm")) {
                if (channel.text("channel_name").startsWith("dm:group:")) "users"
                else "messages-square"
            } else if (channel.flag("is_private")) "lock" else "hash",
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
                if (selected) FontWeight.SemiBold
                else if (channel.text("id") in app.unread) FontWeight.SemiBold else null,
            color =
                if (selected) MaterialTheme.colorScheme.onSecondaryContainer else Color.Unspecified,
        )
        if (channel.text("id") in app.unread)
            Box(
                Modifier.size(8.dp)
                    .clip(RoundedCornerShape(4.dp))
                    .background(MaterialTheme.colorScheme.secondary)
                    .then(webControlModifier("img"))
                    .semantics { contentDescription = app.t("channels.unread") }
            )
        if (channel.present("archive_at"))
            Icon("archive", app.t("moderation.archived"), Modifier.size(12.dp))
    }
}

@Composable
internal fun Welcome(app: Session) {
    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Column(
            Modifier.widthIn(max = 448.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box(
                Modifier.size(48.dp)
                    .clip(RoundedCornerShape(18.dp))
                    .background(MaterialTheme.colorScheme.secondary),
                contentAlignment = Alignment.Center,
            ) {
                BrandLogo(Modifier.fillMaxSize())
            }
            Text(
                app.t("welcome.title"),
                Modifier.then(
                        webControlModifier("heading", attributes = mapOf("aria-level" to "1"))
                    )
                    .semantics { heading() },
                weight = FontWeight.SemiBold,
                size = 20.sp,
                letterSpacing = (-0.5).sp,
            )
            Text(
                app.t("welcome.description"),
                muted = true,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
        }
    }
}

@Composable
internal fun Header(
    app: Session,
    channel: Row,
    users: List<Row>,
    memberships: List<Row>,
    narrow: Boolean,
    wide: Boolean,
    presentation: Row = JsonObject(emptyMap()),
) {
    val id = channel.text("id")
    val manage = canManage(channel, app.config.userId, app.config.isAdmin)
    val count =
        if ("memberCount" in presentation) presentation.text("memberCount")
        else memberships.count { it.text("channel_id") == id }.toString()
    val showCount =
        "memberCount" in presentation ||
            !channel.flag("is_private") ||
            isMember(memberships, id, app.config.userId)
    PaneHeader {
        if (narrow) {
            IconButton("arrow-left", app.t("channels.back"), size = 44.dp) { app.pane = "sidebar" }
            Spacer(Modifier.width(12.dp))
        }
        Box(
            Modifier.size(32.dp)
                .clip(RoundedCornerShape(10.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                if (channel.flag("is_dm")) {
                    if (channel.text("channel_name").startsWith("dm:group:")) "users"
                    else "messages-square"
                } else if (channel.flag("is_private")) "lock" else "hash",
                modifier = Modifier.size(16.dp),
            )
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    presentation.text("displayTitle").ifEmpty {
                        if (channel.flag("is_dm")) dmTitle(app, channel, users, memberships)
                        else channel.text("channel_name")
                    },
                    Modifier.then(
                            webControlModifier("heading", attributes = mapOf("aria-level" to "2"))
                        )
                        .semantics { heading() },
                    weight = FontWeight.SemiBold,
                    maxLines = 1,
                )
                if (channel.present("archive_at") && !channel.flag("is_dm"))
                    ChannelArchiveBadge(app)
            }
            Text(
                presentation.text("subtitle").ifEmpty {
                    channel.text("description").trim().ifEmpty { app.t("channels.noDescription") }
                },
                muted = true,
                small = true,
                maxLines = 1,
            )
        }
        Spacer(Modifier.width(12.dp))
        if (!channel.flag("is_dm"))
            Row(
                Modifier.height(28.dp)
                    .then(webControlModifier("button", expanded = app.membersOpen))
                    .clip(RoundedCornerShape(8.dp))
                    .background(
                        if (app.env.dark) Color.White.copy(alpha = .045f) else Color.Transparent,
                        RoundedCornerShape(8.dp),
                    )
                    .border(
                        1.dp,
                        if (app.env.dark) Color.White.copy(alpha = .15f)
                        else MaterialTheme.colorScheme.outline,
                        RoundedCornerShape(8.dp),
                    )
                    .clickable(role = Role.Button) { app.openMembers(wide) }
                    .semantics {
                        contentDescription =
                            app.t("members.title") +
                                if (showCount && count.isNotEmpty()) " $count" else ""
                    }
                    .padding(horizontal = 11.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    "users",
                    modifier = Modifier.size(14.dp),
                    color = MaterialTheme.colorScheme.onSurface,
                )
                if (!narrow) Text(app.t("members.title"), size = 12.8.sp)
                if (showCount) Text(count.ifEmpty { "…" }, size = 12.8.sp, muted = true)
            }
        Spacer(Modifier.width(4.dp))
        if (manage && !channel.flag("is_dm"))
            IconButton(
                "settings",
                app.t("settings.channelTitle"),
                size = if (narrow) 44.dp else 32.dp,
                color = MaterialTheme.colorScheme.onSurface,
            ) {
                app.dialog = "channel-settings"
            }
        if (channel.flag("is_dm"))
            Menu(
                app.t("dms.menu"),
                listOf(
                    MenuAction("log-out", app.t("dms.leave"), app.config.userId.isNotEmpty()) {
                        app.dialog = "leave"
                    }
                ),
                size = if (narrow) 44.dp else 28.dp,
            )
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
