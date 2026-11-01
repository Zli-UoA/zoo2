package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*

@Composable
internal fun DmItem(
    app: Session,
    channel: Row,
    label: String,
    active: Boolean,
    unread: Boolean,
    narrow: Boolean,
    onOpen: (() -> Unit)? = null,
) {
    val colors = MaterialTheme.colorScheme
    Row(
        Modifier.fillMaxWidth()
            .height(if (narrow) 36.dp else 32.dp)
            .background(
                if (active) colors.secondaryContainer else Color.Transparent,
                RoundedCornerShape(8.dp),
            )
            .then(
                webControlModifier(
                    "link",
                    attributes = if (active) mapOf("aria-current" to "page") else emptyMap(),
                )
            )
            .clickable {
                onOpen?.invoke()
                app.navigate("/channels/${channel.text("id")}")
            }
            .semantics { contentDescription = label }
            .padding(horizontal = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (channel.text("channel_name").startsWith("dm:group:"))
            Box(
                Modifier.size(24.dp).background(colors.surfaceVariant, CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Icon("users", modifier = Modifier.size(14.dp))
            }
        else
            Avatar(
                null,
                size = 24.dp,
                fallback =
                    label
                        .trim()
                        .split(Regex("\\s+"))
                        .take(2)
                        .joinToString("") { it.take(1) }
                        .uppercase(),
            )
        Text(
            label,
            Modifier.weight(1f),
            muted = !active && !unread,
            maxLines = 1,
            weight = if (active || unread) FontWeight.SemiBold else FontWeight.Normal,
        )
        if (unread)
            Box(
                Modifier.size(8.dp)
                    .background(colors.secondary, CircleShape)
                    .then(webControlModifier("img"))
                    .semantics { contentDescription = app.t("channels.unread") }
            )
    }
}

@Composable
internal fun DmSidebar(
    app: Session,
    channels: List<Row>,
    users: List<Row>,
    memberships: List<Row>,
    loading: Boolean,
    error: Boolean,
    usersError: Boolean,
    active: String,
    unread: Set<String>,
    narrow: Boolean,
    onCreate: (() -> Unit)? = null,
    onOpen: (() -> Unit)? = null,
    onReload: (() -> Unit)? = null,
) {
    CollapsibleSection(
        app.t("dms.title"),
        if (loading) "…" else channels.size.toString(),
        icon = "users",
        actionLabel = app.t("dms.createTitle"),
        action = { if (onCreate != null) onCreate() else app.dialog = "dm" },
    ) {
        Column(
            Modifier.fillMaxWidth().padding(start = 8.dp, end = 8.dp, bottom = 8.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            when {
                loading ->
                    repeat(3) {
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
                        app.t("dms.loadError"),
                        Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                        small = true,
                        color = MaterialTheme.colorScheme.error,
                    )
                usersError ->
                    Column(
                        Modifier.padding(8.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(
                            app.t("dms.usersError"),
                            small = true,
                            color = MaterialTheme.colorScheme.error,
                        )
                        Button(
                            app.t("dms.reload"),
                            modifier = Modifier.heightIn(min = 44.dp),
                            variant = "outline",
                            size = "sm",
                        ) {
                            if (onReload != null) onReload() else app.repo.users.refresh()
                        }
                    }
                channels.isEmpty() -> Text(app.t("dms.empty"), Modifier.padding(8.dp), muted = true)
                else ->
                    channels.forEach {
                        DmItem(
                            app,
                            it,
                            dmTitle(app, it, users, memberships),
                            it.text("id") == active,
                            it.text("id") in unread,
                            narrow,
                            onOpen,
                        )
                    }
            }
        }
    }
}

@Composable
internal fun ChannelArchiveBadge(app: Session) {
    Text(
        app.t("moderation.archived"),
        Modifier.background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(4.dp))
            .padding(horizontal = 6.dp, vertical = 2.dp),
        small = true,
        muted = true,
        size = 10.4.sp,
        lineHeight = 15.6.sp,
    )
}

/** Storybook can provide the same saved frontend preference as the React shell. */
internal val LocalFrontendMode = compositionLocalOf { "compose" }

@Composable
internal fun RailLink(app: Session, url: String, icon: String, label: String) {
    val interactions = remember {
        androidx.compose.foundation.interaction.MutableInteractionSource()
    }
    val hovered by interactions.collectIsHoveredAsState()
    var focused by remember { mutableStateOf(false) }
    val colors = MaterialTheme.colorScheme
    val active = app.env.path.substringBefore('?') == url.substringBefore('?')
    val foreground =
        if (active) colors.onSecondaryContainer
        else if (hovered) colors.onBackground else colors.onSurfaceVariant
    val ring = if (app.env.dark) Color(0xffbdbdbd) else Color(0xff737373)
    NativeLink(
        url,
        Modifier.size(40.dp)
            .onFocusChanged { focused = it.isFocused }
            .hoverable(interactions)
            .focusVisibleRing(14.dp, ring.copy(alpha = .5f), focused && FocusInputModality.keyboard)
            .background(
                if (active) colors.secondaryContainer
                else if (hovered) colors.surfaceVariant else Color.Transparent,
                RoundedCornerShape(14.dp),
            )
            .semantics { contentDescription = label },
        attributes =
            mapOf("title" to label) + if (active) mapOf("data-status" to "active") else emptyMap(),
    ) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Icon(icon, modifier = Modifier.size(20.dp), color = foreground)
        }
    }
}

@Composable
internal fun FrontendSwitch(app: Session) {
    val compose = LocalFrontendMode.current == "compose"
    val label = app.t(if (compose) "experimental.useReact" else "experimental.useCompose")
    val interactions = remember {
        androidx.compose.foundation.interaction.MutableInteractionSource()
    }
    val hovered by interactions.collectIsHoveredAsState()
    var focused by remember { mutableStateOf(false) }
    val colors = MaterialTheme.colorScheme
    val ring = if (app.env.dark) Color(0xffbdbdbd) else Color(0xff737373)
    Box(
        Modifier.size(28.dp)
            .onFocusChanged { focused = it.isFocused }
            .hoverable(interactions)
            .focusVisibleRing(8.dp, ring.copy(alpha = .5f), focused && FocusInputModality.keyboard)
            .background(
                if (hovered) colors.surfaceVariant else Color.Transparent,
                RoundedCornerShape(8.dp),
            )
            .then(
                webControlModifier(
                    "button",
                    attributes =
                        mapOf(
                            "data-frontend-switch" to "",
                            "data-testid" to "frontend-switch",
                            "title" to label,
                        ),
                )
            )
            .clickable(role = androidx.compose.ui.semantics.Role.Button) {
                app.platform.action(if (compose) "react" else "compose")
            }
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Text(if (compose) "R" else "K", weight = FontWeight.Medium)
    }
}
