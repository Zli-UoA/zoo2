package zoo

import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.layoutId
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.*
import kotlin.math.ceil
import kotlin.math.floor
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

private fun notificationGroupKey(rows: List<Row>): String {
    val row = rows.first()
    return if (row.text("notification_type") == "reaction")
        "reaction:${row.text("user_id")}:${row.text("channel_id")}:${row.text("message_id")}"
    else "notification:${row.text("id")}"
}

@Composable
fun Notifications(app: Session, notifications: List<Row>, users: List<Row>) {
    val groups = notificationGroups(notifications)
    val initialLoading by app.repo.notifications.loading.collectAsState()
    val loadError by app.repo.notifications.error.collectAsState()
    val usersLoading by app.repo.users.loading.collectAsState()
    val channelsLoading by app.repo.channels.loading.collectAsState()
    val contextLoading = remember { mutableStateMapOf<String, Boolean>() }
    val contextCache = remember(app) { mutableStateMapOf<String, Row>() }

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
    val loading =
        loadError == null &&
            (initialLoading ||
                usersLoading ||
                channelsLoading ||
                (!hasLoaded && groups.isNotEmpty() && visible.isEmpty()) ||
                groups.any {
                    notificationGroupKey(it) in visible &&
                        contextLoading[it.first().text("id")] != false
                })
    LaunchedEffect(loading, visible) {
        if (!loading && (groups.isEmpty() || visible.isNotEmpty())) hasLoaded = true
    }
    val initialMask = loading && !hasLoaded
    Box(
        Modifier.fillMaxSize().then(webControlModifier("region")).semantics {
            contentDescription = app.t("notifications.title")
        }
    ) {
        Column(Modifier.fillMaxSize()) {
            ScreenHeader("bell", app.t("notifications.title"), primaryIcon = true)
            HorizontalDivider()
            if (loadError != null) {
                Text(
                    app.t("notifications.loadError"),
                    Modifier.padding(24.dp),
                    color = MaterialTheme.colorScheme.error,
                )
            }
            if (loadError == null)
                Box(Modifier.weight(1f)) {
                    LazyColumn(
                        Modifier.fillMaxSize()
                            .pointerInput(initialMask) {
                                if (initialMask)
                                    awaitPointerEventScope {
                                        while (true) {
                                            val event = awaitPointerEvent(PointerEventPass.Initial)
                                            event.changes.forEach { it.consume() }
                                        }
                                    }
                            }
                            .focusProperties { onEnter = { if (initialMask) cancelFocusChange() } }
                            .focusGroup()
                            .then(webControlModifier("list"))
                            .alpha(if (initialMask) 0f else 1f)
                            .semantics {
                                contentDescription = app.t("notifications.title")
                                if (initialMask) invisibleToUser()
                            },
                        state = scroll,
                        userScrollEnabled = !initialMask,
                        contentPadding = PaddingValues(0.dp),
                    ) {
                        items(groups, key = ::notificationGroupKey) { rows ->
                            NotificationCard(
                                app,
                                rows,
                                users,
                                notificationGroupKey(rows) in visible,
                                !loading,
                                onLoading = { contextLoading[rows.first().text("id")] = it },
                                contextCache = contextCache,
                                interactive = !initialMask,
                            )
                        }
                    }
                    if (groups.isEmpty() && !initialLoading)
                        Column(
                            Modifier.fillMaxWidth().heightIn(min = 288.dp).padding(24.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                            verticalArrangement = Arrangement.Center,
                        ) {
                            Icon("bell", modifier = Modifier.size(32.dp))
                            Spacer(Modifier.height(12.dp))
                            Text(app.t("notifications.empty"), muted = true)
                        }
                    if (loading && !initialMask)
                        Box(
                            Modifier.fillMaxWidth()
                                .align(Alignment.BottomCenter)
                                .background(MaterialTheme.colorScheme.background)
                                .padding(vertical = 16.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            NotificationSpinner(app)
                        }
                }
        }
        if (initialMask)
            Box(
                Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background),
                contentAlignment = Alignment.Center,
            ) {
                NotificationSpinner(app)
            }
    }
}

@Composable
private fun NotificationSpinner(app: Session) {
    val angle =
        if (app.env.reducedMotion) 0f
        else {
            val transition = rememberInfiniteTransition()
            val animated by
                transition.animateFloat(
                    0f,
                    360f,
                    infiniteRepeatable(tween(1000, easing = LinearEasing)),
                )
            animated
        }
    Box(
        Modifier.then(webControlModifier("status")).semantics {
            contentDescription = app.t("common.loading")
            liveRegion = LiveRegionMode.Polite
        }
    ) {
        Icon("loader-circle", modifier = Modifier.size(28.dp).graphicsLayer { rotationZ = angle })
    }
}

@Composable
fun NotificationCard(
    app: Session,
    rows: List<Row>,
    users: List<Row>,
    visible: Boolean,
    readEnabled: Boolean,
    onLoading: (Boolean) -> Unit,
    channelOverride: Row? = null,
    channelProvided: Boolean = false,
    membershipsOverride: List<Row>? = null,
    onRead: ((List<String>) -> Unit)? = null,
    contextCache: MutableMap<String, Row>? = null,
    interactive: Boolean = true,
) {
    val row = rows.first()
    val id = row.text("id")
    val actors = rows.map { it.text("actor_id") }.distinct()
    val user = users.find { it.text("id") == row.text("actor_id") }
    val channels by app.repo.channels.rows.collectAsState()
    val repoMemberships by app.repo.members.rows.collectAsState()
    val memberships = membershipsOverride ?: repoMemberships
    val channel =
        if (channelProvided) channelOverride
        else channels.find { it.text("id") == row.text("channel_id") }
    var context by remember(id) { mutableStateOf<Row?>(contextCache?.get(id)) }
    var loading by remember(id) { mutableStateOf(context == null) }
    var wasVisible by remember(id) { mutableStateOf(false) }
    LaunchedEffect(visible) { if (visible) wasVisible = true }
    LaunchedEffect(id, wasVisible, app.env.online, channel) {
        if (wasVisible && channel != null && context == null) {
            loading = true
            onLoading(true)
            try {
                context = app.api.request("/api/notifications/$id/context").objectOrEmpty()
                contextCache?.set(id, context!!)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {} finally {
                loading = false
                onLoading(false)
            }
        } else if (wasVisible) {
            loading = false
            onLoading(false)
        }
    }
    var locallyRead by remember(id) { mutableStateOf(false) }
    val read = locallyRead || rows.all { it.flag("is_read") }
    val message = context?.get("message") as? JsonObject
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
        interactive,
    ) {
        if (
            !read &&
                !attempted &&
                readEnabled &&
                interactive &&
                app.env.online &&
                app.env.visible &&
                app.env.focused &&
                visible
        ) {
            delay(500)
            attempted = true
            if (onRead != null) {
                onRead(rows.filterNot { it.flag("is_read") }.map { it.text("id") })
                return@LaunchedEffect
            }
            locallyRead = true
            rows
                .filterNot { it.flag("is_read") }
                .forEach {
                    try {
                        app.api.request("/api/notifications/${it.text("id")}/read", "PATCH", obj())
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        locallyRead = false
                        app.toast = app.t("notifications.readError")
                    }
                }
        }
    }
    val notificationLabel =
        app.t(
            if (row.text("notification_type") == "reaction" && actors.size > 1)
                "notifications.reactionGroup"
            else
                when (row.text("notification_type")) {
                    "reaction" -> "notifications.reaction"
                    "mention" -> "notifications.mention"
                    "dm" -> "notifications.dm"
                    else -> "notifications.reply"
                },
            "actor" to actor,
            "count" to (actors.size - 1).toString(),
        )
    BoxWithConstraints(
        Modifier.fillMaxWidth().then(webControlModifier("listitem")).semantics {
            contentDescription = notificationLabel
            isTraversalGroup = true
        }
    ) {
        val mobile = maxWidth < 640.dp
        val colors = MaterialTheme.colorScheme
        val type = row.text("notification_type")
        val icon =
            if (type == "reaction") "heart"
            else if (type == "mention") "at-sign" else "message-circle"
        val contextKey =
            when (type) {
                "reaction" -> "reactionContext"
                "mention" -> "mentionContext"
                "dm" -> "dmContext"
                else -> "replyContext"
            }
        val label =
            if (channel == null) app.t("notifications.unavailable")
            else if (channel.flag("is_dm")) dmTitle(app, channel, users, memberships)
            else "#${channel.text("channel_name")}"
        val rowInteractions = remember { MutableInteractionSource() }
        val rowHovered by rowInteractions.collectIsHoveredAsState()
        Row(
            Modifier.fillMaxWidth()
                .hoverable(rowInteractions)
                .background(
                    if (rowHovered) colors.surfaceVariant.copy(alpha = .4f) else Color.Transparent
                )
                .then(
                    if (interactive && channel != null && message != null)
                        Modifier.nativeNavigation(destination, role = null)
                            .pointerHoverIcon(PointerIcon.Hand)
                    else Modifier
                )
                .padding(
                    horizontal = if (mobile) 16.dp else 20.dp,
                    vertical = if (mobile) 20.dp else 12.dp,
                ),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box {
                Avatar(user, size = if (mobile) 40.dp else 36.dp)
                Box(
                    Modifier.align(Alignment.BottomEnd)
                        .offset(4.dp, 4.dp)
                        .size(20.dp)
                        .drawBehind {
                            drawCircle(
                                colors.background,
                                radius = size.minDimension / 2 + 2.dp.toPx(),
                            )
                        }
                        .background(colors.surfaceVariant, CircleShape),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(icon, modifier = Modifier.size(12.dp))
                }
            }
            Column(
                Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(if (mobile) 8.dp else 4.dp),
            ) {
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
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
                            Modifier.weight(1f, fill = false).alignByBaseline(),
                            weight = FontWeight.SemiBold,
                            maxLines = 1,
                        )
                        if (user?.flag("is_bot") == true)
                            Text(
                                "APP",
                                Modifier.background(colors.surfaceVariant, RoundedCornerShape(4.dp))
                                    .padding(horizontal = 4.dp)
                                    .alignByBaseline(),
                                size = 10.sp,
                                lineHeight = 15.sp,
                                muted = true,
                                weight = FontWeight.Medium,
                            )
                        CompositionLocalProvider(
                            LocalMfmStyle provides TextStyle(fontFeatureSettings = "tnum")
                        ) {
                            Text(
                                app.platform.notificationTime(row.text("created_at")),
                                Modifier.alignByBaseline(),
                                muted = true,
                                size = 11.sp,
                                lineHeight = 16.5.sp,
                                maxLines = 1,
                            )
                        }
                    }
                    Box(Modifier.width(56.dp), contentAlignment = Alignment.CenterEnd) {
                        if (!read)
                            Text(
                                app.t("notifications.unreadStatus"),
                                Modifier.background(colors.primary, RoundedCornerShape(50))
                                    .padding(horizontal = 8.dp, vertical = 2.dp),
                                color = colors.onPrimary,
                                weight = FontWeight.SemiBold,
                                size = 10.sp,
                                lineHeight = 15.sp,
                            )
                        else
                            Icon(
                                "check-check",
                                app.t("notifications.readStatus"),
                                Modifier.size(16.dp),
                            )
                    }
                }
                if (message != null && channel != null)
                    Box(Modifier.fillMaxWidth()) {
                        CompositionLocalProvider(
                            LocalMfmStyle provides TextStyle(lineHeight = 22.75.sp),
                            LocalInlineLinksEnabled provides interactive,
                        ) {
                            RichText(
                                quotePreview(message.text("content")),
                                users,
                                maxLines = 3,
                                selection = interactive,
                            )
                        }
                    }
                else
                    Text(
                        app.t(
                            if (loading && channel != null) "common.loading"
                            else "notifications.messageUnavailable"
                        ),
                        muted = true,
                    )
                @Composable
                fun destinationContent() {
                    NotificationDestination(
                        if (channel?.flag("is_dm") == true)
                            if (channel.text("channel_name").startsWith("dm:group:")) "users"
                            else "user-round"
                        else icon,
                        app.t("notifications.$contextKey").takeIf { it != label },
                        label,
                        channel != null && message != null,
                    )
                }
                val destinationInteractions = remember { MutableInteractionSource() }
                val destinationHovered by destinationInteractions.collectIsHoveredAsState()
                var destinationFocused by remember { mutableStateOf(false) }
                if (interactive && channel != null && message != null)
                    NativeLink(
                        destination,
                        Modifier.hoverable(destinationInteractions)
                            .onFocusChanged { destinationFocused = it.isFocused }
                            .then(
                                if (destinationFocused)
                                    Modifier.border(
                                        2.dp,
                                        colors.primary.copy(alpha = .5f),
                                        RoundedCornerShape(4.dp),
                                    )
                                else Modifier
                            )
                            .semantics {
                                contentDescription =
                                    (if (app.t("notifications.$contextKey") != label)
                                        "${app.t("notifications.$contextKey")} · "
                                    else "") +
                                        label +
                                        " — " +
                                        notificationLabel +
                                        ": " +
                                        quotePreview(message.text("content"))
                            },
                        attributes = mapOf("title" to label),
                    ) {
                        CompositionLocalProvider(
                            LocalMfmStyle provides
                                TextStyle(
                                    textDecoration =
                                        if (destinationHovered) TextDecoration.Underline
                                        else TextDecoration.None
                                )
                        ) {
                            destinationContent()
                        }
                    }
                else destinationContent()
                val reactionGroups =
                    if (type == "reaction")
                        context
                            ?.get("reactions")
                            ?.rows()
                            ?.filter { it.text("user_id") in actors }
                            ?.groupBy { reactionKey(it.text("emoji")) } ?: emptyMap()
                    else emptyMap()
                if (reactionGroups.isNotEmpty())
                    FlowRow(
                        Modifier.padding(top = 2.dp).then(webControlModifier("list")).semantics {
                            contentDescription = app.t("notifications.reactions")
                        },
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        reactionGroups.forEach { (emoji, reactions) ->
                            Row(
                                Modifier.background(colors.surfaceVariant, RoundedCornerShape(50))
                                    .padding(horizontal = 10.dp, vertical = 4.dp)
                                    .then(
                                        webControlModifier(
                                            "listitem",
                                            attributes = mapOf("title" to emoji),
                                        )
                                    )
                                    .semantics {
                                        contentDescription =
                                            "$emoji ${reactions.map { it.text("user_id") }.distinct().size}"
                                    },
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                ReactionEmoji(emoji, Modifier.size(20.dp))
                                CompositionLocalProvider(
                                    LocalMfmStyle provides TextStyle(fontFeatureSettings = "tnum")
                                ) {
                                    Text(
                                        reactions
                                            .map { it.text("user_id") }
                                            .distinct()
                                            .size
                                            .toString(),
                                        small = true,
                                        muted = true,
                                    )
                                }
                            }
                        }
                    }
            }
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

/** Shared rail notification link; the catalog renders the same component. */
@Composable
fun NotificationsButton(app: Session, unread: Int) {
    val interactions = remember { MutableInteractionSource() }
    val hovered by interactions.collectIsHoveredAsState()
    var focused by remember { mutableStateOf(false) }
    val active = app.env.path.substringBefore('?') == "/notifications"
    val colors = MaterialTheme.colorScheme
    val shape = RoundedCornerShape(12.dp)
    val foreground =
        when {
            active -> colors.onSecondaryContainer
            hovered -> colors.onBackground
            else -> colors.onSurfaceVariant
        }
    NativeLink(
        "/notifications",
        Modifier.size(40.dp)
            .onFocusChanged { focused = it.isFocused }
            .hoverable(interactions)
            .then(
                if (focused) Modifier.border(3.dp, colors.primary.copy(alpha = .5f), shape)
                else Modifier
            )
            .background(
                when {
                    active -> colors.secondaryContainer
                    hovered -> colors.surfaceVariant
                    else -> Color.Transparent
                },
                shape,
            )
            .semantics {
                contentDescription = app.t("notifications.unread", "count" to unread.toString())
            },
        attributes = mapOf("title" to app.t("notifications.title")),
    ) {
        Box(Modifier.size(40.dp), contentAlignment = Alignment.Center) {
            Icon("bell", modifier = Modifier.size(20.dp), color = foreground)
            if (unread > 0)
                Box(
                    Modifier.align(Alignment.TopEnd)
                        .offset(x = 4.dp, y = (-4).dp)
                        .clip(CircleShape)
                        .background(colors.error)
                        .padding(horizontal = 4.dp)
                        .clearAndSetSemantics {},
                    contentAlignment = Alignment.Center,
                ) {
                    CompositionLocalProvider(
                        LocalMfmStyle provides TextStyle(fontFeatureSettings = "tnum")
                    ) {
                        Text(
                            if (unread > 99) "99+" else unread.toString(),
                            size = 10.sp,
                            lineHeight = 15.sp,
                            color = Color.White,
                        )
                    }
                }
        }
    }
}

/** CSS flex-shrink distributes overflow proportionally between both labels. */
@Composable
private fun NotificationDestination(
    icon: String,
    context: String?,
    label: String,
    available: Boolean,
) {
    val textMeasurer = rememberTextMeasurer()
    val fonts = LocalFonts.current
    val textStyle = MaterialTheme.typography.bodySmall.merge(LocalMfmStyle.current)
    fun advance(text: String, weight: FontWeight? = null): Float =
        textMeasurer
            .measure(fontText(text, fonts, weight), textStyle, softWrap = false)
            .getLineRight(0)
    val contextAdvance = context?.let { advance(it) } ?: 0f
    val labelAdvance = advance(label, FontWeight.Medium)
    val dotAdvance = advance("·")
    Layout(
        content = {
            Icon(icon, modifier = Modifier.size(16.dp).layoutId("icon"))
            if (context != null) {
                Text(
                    context,
                    Modifier.layoutId("context"),
                    small = true,
                    muted = true,
                    maxLines = 1,
                )
                Text("·", Modifier.layoutId("dot"), small = true, muted = true)
            }
            Text(
                label,
                Modifier.layoutId("label"),
                small = true,
                muted = true,
                weight = FontWeight.Medium,
                maxLines = 1,
            )
            if (available) Icon("arrow-up-right", modifier = Modifier.size(14.dp).layoutId("arrow"))
        }
    ) { measurables, constraints ->
        val gap = 6.dp.toPx()
        val bases =
            measurables.map {
                when (it.layoutId) {
                    "context" -> contextAdvance
                    "label" -> labelAdvance
                    "dot" -> dotAdvance
                    "icon" -> 16.dp.toPx()
                    else -> 14.dp.toPx()
                }
            }
        val natural = bases.sum() + gap * (bases.size - 1)
        val width = constraints.constrainWidth(ceil(natural).toInt())
        val logicalWidth = minOf(natural, constraints.maxWidth.toFloat())
        val flexible =
            measurables.indices.filter { measurables[it].layoutId in listOf("context", "label") }
        val fixed =
            measurables.indices
                .filterNot { it in flexible }
                .sumOf { bases[it].toDouble() }
                .toFloat()
        val remaining = (logicalWidth - fixed - gap * (bases.size - 1)).coerceAtLeast(0f)
        val total = flexible.sumOf { bases[it].toDouble() }.toFloat()
        val widths = bases.toMutableList()
        if (total > remaining) flexible.forEach { widths[it] = bases[it] * remaining / total }
        val children =
            measurables.mapIndexed { index, child ->
                child.measure(Constraints.fixedWidth(ceil(widths[index]).toInt()))
            }
        val height = constraints.constrainHeight(children.maxOfOrNull { it.height } ?: 0)
        layout(width, height) {
            var x = 0f
            children.forEachIndexed { index, child ->
                val pixelX = floor(x).toInt()
                val fraction = x - pixelX
                child.placeRelativeWithLayer(pixelX, (height - child.height) / 2) {
                    translationX = fraction
                }
                x += widths[index] + gap
            }
        }
    }
}
