package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.interaction.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.*
import androidx.compose.ui.graphics.*
import androidx.compose.ui.input.key.*
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import kotlinx.serialization.json.*
import org.jetbrains.compose.resources.decodeToImageBitmap

@Composable
fun Avatar(
    user: Row?,
    modifier: Modifier = Modifier,
    size: Dp = 32.dp,
    fallback: String? = null,
    badge: Boolean = false,
) {
    val name = user?.let(::author).orEmpty().ifEmpty { "?" }
    val colors = MaterialTheme.colorScheme
    Box(modifier.size(size)) {
        BoxWithConstraints(
            Modifier.matchParentSize()
                .clip(CircleShape)
                .background(colors.surfaceVariant)
                .border(1.dp, colors.outline, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            val url = user?.text("avatar_url").orEmpty()
            if (url.isNotEmpty()) RemoteImage(url, name, Modifier.fillMaxSize())
            else if (user?.flag("is_bot") == true) Icon("bot", modifier = Modifier.size(16.dp))
            else
                Text(
                    fallback
                        ?: name
                            .trim()
                            .split(Regex("\\s+"))
                            .take(2)
                            .joinToString("") { it.take(1) }
                            .uppercase(),
                    small = true,
                    size = if (maxWidth > 24.dp) 14.sp else 12.sp,
                    lineHeight = if (maxWidth > 24.dp) 20.sp else 16.sp,
                    color = colors.onSurfaceVariant,
                )
        }
        if (badge)
            Box(
                Modifier.align(Alignment.BottomEnd)
                    .size(10.dp)
                    .drawBehind {
                        drawCircle(
                            colors.background,
                            radius = this.size.minDimension / 2 + 2.dp.toPx(),
                        )
                    }
                    .background(colors.primary, CircleShape)
            )
    }
}

@Composable
fun RemoteImage(
    url: String,
    label: String,
    modifier: Modifier = Modifier,
    onLoaded: (() -> Unit)? = null,
    onError: (() -> Unit)? = null,
) {
    val app = LocalApp.current
    var bitmap by remember(url) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(url) {
        try {
            if (
                url.startsWith("https://") ||
                    url.startsWith("http://") ||
                    (url.startsWith("/") && !url.startsWith("//"))
            ) {
                bitmap = app.image(url).decodeToImageBitmap()
                onLoaded?.invoke()
            }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            onError?.invoke()
        }
    }
    bitmap?.let { Image(it, label, modifier) }
        ?: Box(
            modifier.background(MaterialTheme.colorScheme.surfaceVariant),
            contentAlignment = Alignment.Center,
        ) {
            Text(label, small = true, maxLines = 1)
        }
}

@Composable
fun Chat(
    app: Session,
    channel: Row,
    users: List<Row>,
    members: List<Row>,
    wide: Boolean,
    modifier: Modifier = Modifier,
    membershipOverride: Boolean? = null,
) {
    val id = channel.text("id")
    val table = remember(id) { app.repo.messages(id) }
    DisposableEffect(id) { onDispose { app.repo.releaseChannel(id) } }
    val all by table.rows.collectAsState()
    val loading by table.loading.collectAsState()
    val error by table.error.collectAsState()
    val reactions by app.repo.reactions(id).rows.collectAsState()
    val joined = membershipOverride ?: isMember(members, id, app.config.userId)
    val canPost = joined && !channel.present("archive_at") && app.env.online
    LaunchedEffect(id, app.env.visible, app.env.online) {
        if (app.env.online && app.env.visible) {
            try {
                app.api.request(
                    "/api/channels/$id/view",
                    "POST",
                    buildJsonObject { put("open", true) },
                )
            } catch (_: Exception) {}
            try {
                awaitCancellation()
            } finally {
                withContext(NonCancellable) {
                    try {
                        app.api.request(
                            "/api/channels/$id/view",
                            "POST",
                            buildJsonObject { put("open", false) },
                        )
                    } catch (_: Exception) {}
                }
            }
        }
    }
    LaunchedEffect(id, app.query["message"], all.size) {
        val target = app.query["message"]
        all.find { it.text("id") == target }
            ?.let { if (it.present("parent_id")) app.openThread(it.text("parent_id"), wide) }
    }
    val attachmentRows by app.repo.attachments(id).rows.collectAsState()
    CompositionLocalProvider(LocalAttachments provides attachmentRows) {
        Row(modifier.fillMaxWidth()) {
            if (wide || app.thread == null)
                Column(Modifier.weight(1f).fillMaxHeight()) {
                    MessageList(
                        app,
                        channel,
                        all,
                        reactions,
                        users,
                        null,
                        wide,
                        Modifier.weight(1f),
                        isLoading = loading,
                        isError = error != null,
                        canCompose = canPost,
                        retry = { table.refresh() },
                    )
                    when {
                        channel.present("archive_at") ->
                            Text(
                                app.t("moderation.archivedReadOnly"),
                                Modifier.padding(16.dp),
                                muted = true,
                            )
                        !joined -> JoinToPostPrompt(app, channel)
                        else -> Composer(app, id, null, users, canPost)
                    }
                }
            app.thread?.let { parent ->
                if (wide) VerticalDivider()
                ThreadSurface(
                    app,
                    visibleMessages(all, parent).size,
                    loading,
                    close = { app.thread = null },
                    root = {
                        all.find { it.text("id") == parent && !it.present("deleted_at") }
                            ?.let {
                                Message(
                                    app,
                                    channel,
                                    it,
                                    all,
                                    reactions,
                                    users,
                                    false,
                                    wide,
                                    canOpenThread = false,
                                )
                            }
                    },
                    replies = {
                        MessageList(
                            app,
                            channel,
                            all,
                            reactions,
                            users,
                            parent,
                            wide,
                            Modifier.fillMaxSize(),
                            isLoading = loading,
                            isError = error != null,
                            canCompose = canPost,
                            retry = { table.refresh() },
                        )
                    },
                    composer =
                        if (joined && !channel.present("archive_at")) {
                            { Composer(app, id, parent, users, canPost) }
                        } else null,
                )
            }
        }
    }
}

@Composable
private fun JoinToPostPrompt(app: Session, channel: Row) {
    val id = channel.text("id")
    val colors = MaterialTheme.colorScheme
    Column(Modifier.fillMaxWidth()) {
        HorizontalDivider()
        Column(
            Modifier.padding(12.dp)
                .fillMaxWidth()
                .background(colors.surfaceVariant.copy(alpha = .3f), RoundedCornerShape(14.dp))
                .drawBehind {
                    drawRoundRect(
                        colors.outline,
                        androidx.compose.ui.geometry.Offset(.5.dp.toPx(), .5.dp.toPx()),
                        androidx.compose.ui.geometry.Size(
                            size.width - 1.dp.toPx(),
                            size.height - 1.dp.toPx(),
                        ),
                        androidx.compose.ui.geometry.CornerRadius(13.5.dp.toPx()),
                        style =
                            androidx.compose.ui.graphics.drawscope.Stroke(
                                1.dp.toPx(),
                                pathEffect =
                                    PathEffect.dashPathEffect(
                                        floatArrayOf(3.dp.toPx(), 3.dp.toPx())
                                    ),
                            ),
                    )
                }
                .padding(17.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                app.t(
                    if (channel.flag("is_private")) "channels.privateMessages"
                    else "messages.joinToPostDescription"
                ),
                muted = true,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
            if (!channel.flag("is_private"))
                Button(
                    app.t("messages.joinToPost"),
                    app.env.online && "join:$id" !in app.pending,
                    primary = true,
                    size = "sm",
                ) {
                    app.run("join:$id") {
                        val row = obj("channel_id" to id, "user_id" to app.config.userId)
                        app.repo.members.write(
                            app.api,
                            "/api/channels/$id/join",
                            "POST",
                            obj(),
                            row,
                        )
                    }
                }
        }
    }
}

@Composable
internal fun MessageList(
    app: Session,
    channel: Row,
    all: List<Row>,
    reactions: List<Row>,
    users: List<Row>,
    parent: String?,
    wide: Boolean,
    modifier: Modifier,
    isLoading: Boolean = false,
    isError: Boolean = false,
    canCompose: Boolean = true,
    editingId: String? = null,
    editingText: String? = null,
    canReact: Boolean = true,
    retry: (() -> Unit)? = null,
    readState: Boolean = true,
    callbacks: ((Row) -> MessageCallbacks)? = null,
) {
    val messages = visibleMessages(all, parent)
    var readBoundary by remember(channel.text("id")) { mutableStateOf<Long?>(null) }
    var readReady by remember(channel.text("id")) { mutableStateOf(false) }
    LaunchedEffect(channel.text("id"), readState, parent) {
        if (readState && parent == null) {
            try {
                val initial =
                    app.api.request("/api/channels/${channel.text("id")}/read-state").jsonObject
                readBoundary = timestampMillis(initial.text("last_read_at")) ?: 0L
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {}
        }
        readReady = true
    }
    val firstUnreadId =
        if (readState && parent == null)
            readBoundary?.let { boundary ->
                messages
                    .firstOrNull {
                        it.text("user_id") != app.config.userId &&
                            (timestampMillis(it.text("created_at")) ?: 0L) > boundary
                    }
                    ?.text("id")
            }
        else null
    val state = rememberLazyListState()
    LaunchedEffect(app.query["message"], messages.size) {
        val index = messages.indexOfFirst { it.text("id") == app.query["message"] }
        if (index >= 0) state.scrollToItem(index * 2 + 1)
    }
    var previousSize by remember(channel.text("id"), parent) { mutableStateOf(0) }
    var pinned by remember(channel.text("id"), parent) { mutableStateOf(true) }
    var newBelow by remember(channel.text("id"), parent) { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    suspend fun bottom() {
        if (messages.isNotEmpty()) {
            state.scrollToItem(messages.lastIndex * 2 + 1)
            state.scrollBy(state.layoutInfo.visibleItemsInfo.lastOrNull()?.size?.toFloat() ?: 0f)
        }
        pinned = true
        newBelow = false
    }
    LaunchedEffect(state) {
        snapshotFlow { state.isScrollInProgress }
            .collect { moving ->
                if (!moving) {
                    pinned = !state.canScrollForward
                    if (pinned) newBelow = false
                }
            }
    }
    LaunchedEffect(messages.size) {
        if (messages.isNotEmpty() && messages.size > previousSize) {
            if (previousSize == 0 || pinned || messages.last().text("user_id") == app.config.userId)
                bottom()
            else newBelow = true
        }
        previousSize = messages.size
    }
    // LazyColumn retains the first visible key/offset while an earlier card grows.
    // Only a reader already following the end follows asynchronously resized cards.
    LaunchedEffect(state, messages.size) {
        snapshotFlow { state.layoutInfo.visibleItemsInfo.map { it.key to it.size } }
            .collect { if (pinned && !state.isScrollInProgress && state.canScrollForward) bottom() }
    }
    LaunchedEffect(
        channel.text("id"),
        parent,
        app.env.visible,
        app.env.online,
        all.size,
        readReady,
    ) {
        if (readReady && readState && parent == null && app.env.visible && app.env.online) {
            var lastRead: String? = null
            snapshotFlow { state.layoutInfo.visibleItemsInfo.map { it.key.toString() } }
                .collect { keys ->
                    val last = visibleMessages(all).lastOrNull { it.text("id") in keys }
                    if (last != null && last.text("id") != lastRead) {
                        try {
                            app.api.request(
                                "/api/channels/${channel.text("id")}/read-state",
                                "POST",
                                obj("message_id" to last.text("id")),
                            )
                            lastRead = last.text("id")
                            if (last == visibleMessages(all).lastOrNull())
                                app.unread = app.unread - channel.text("id")
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {}
                    }
                }
        }
    }
    Box(modifier.fillMaxWidth()) {
        if (parent != null && messages.isEmpty()) {
            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    app.t(
                        if (isLoading) "threads.repliesLoading"
                        else if (isError) "threads.loadError" else "threads.empty"
                    ),
                    size = 12.sp,
                    muted = !isError,
                    color =
                        if (isError) MaterialTheme.colorScheme.error
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (isError && retry != null)
                    Button(app.t("offline.retry"), variant = "outline", size = "sm", click = retry)
            }
        } else if (isLoading && messages.isEmpty()) MessageLoadingSkeleton()
        else if (isError && messages.isEmpty())
            MessageEmptyState(app, "messages.loadError", "messages.retryHelp", true, retry)
        else
            LazyColumn(
                Modifier.fillMaxSize(),
                state = state,
                contentPadding =
                    PaddingValues(horizontal = 8.dp, vertical = if (parent != null) 8.dp else 12.dp),
            ) {
                if (messages.isEmpty())
                    item {
                        Box(Modifier.fillParentMaxSize(), contentAlignment = Alignment.Center) {
                            Column(
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Text(app.t("messages.empty"), weight = FontWeight.Medium)
                                if (canCompose)
                                    Text(app.t("messages.first"), small = true, muted = true)
                            }
                        }
                    }
                messages.forEachIndexed { index, row ->
                    val previous = messages.getOrNull(index - 1)
                    item("date:${row.text("id")}") {
                        if (
                            parent == null &&
                                (previous == null ||
                                    app.platform.day(previous.text("created_at")) !=
                                        app.platform.day(row.text("created_at")))
                        )
                            Row(
                                Modifier.fillMaxWidth().padding(vertical = 8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(12.dp),
                            ) {
                                HorizontalDivider(Modifier.weight(1f))
                                Box(
                                    Modifier.height(23.dp)
                                        .border(
                                            1.dp,
                                            MaterialTheme.colorScheme.outline,
                                            RoundedCornerShape(12.dp),
                                        )
                                        .padding(horizontal = 10.dp),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Text(
                                        app.dayLabel(row.text("created_at")),
                                        small = true,
                                        muted = true,
                                        size = 11.sp,
                                        weight = FontWeight.Medium,
                                    )
                                }
                                HorizontalDivider(Modifier.weight(1f))
                            }
                    }
                    item(row.text("id")) {
                        if (row.text("id") == firstUnreadId)
                            Row(
                                Modifier.fillMaxWidth()
                                    .then(webControlModifier("separator"))
                                    .semantics { contentDescription = app.t("messages.new") }
                                    .padding(vertical = 8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                HorizontalDivider(
                                    Modifier.weight(1f),
                                    color = MaterialTheme.colorScheme.error,
                                )
                                Text(
                                    app.t("messages.new"),
                                    small = true,
                                    weight = FontWeight.SemiBold,
                                    color = MaterialTheme.colorScheme.error,
                                )
                                HorizontalDivider(
                                    Modifier.width(24.dp),
                                    color = MaterialTheme.colorScheme.error,
                                )
                            }
                        Message(
                            app,
                            channel,
                            row,
                            all,
                            reactions,
                            users,
                            row.text("id") != firstUnreadId && groupedWith(previous, row),
                            wide,
                            initialEditing = editingId == row.text("id"),
                            initialEditingText = editingText.takeIf { editingId == row.text("id") },
                            canReact = canReact,
                            callbacks = callbacks?.invoke(row),
                            groupEnd =
                                messages.getOrNull(index + 1)?.let { !groupedWith(row, it) } ?: true,
                        )
                    }
                }
            }
        if (newBelow)
            Button(
                app.t("messages.new"),
                modifier = Modifier.align(Alignment.BottomCenter).padding(12.dp),
                primary = true,
            ) {
                scope.launch { bottom() }
            }
    }
}

class MessageCallbacks(
    val startEdit: (() -> Unit)? = null,
    val cancelEdit: (() -> Unit)? = null,
    val saveEdit: (() -> Unit)? = null,
    val requestDelete: (() -> Unit)? = null,
    val openThread: (() -> Unit)? = null,
    val pickReaction: (() -> Unit)? = null,
    val reaction: ((String) -> Unit)? = null,
    val changeText: ((String) -> Unit)? = null,
)

@Composable
internal fun Message(
    app: Session,
    channel: Row,
    message: Row,
    all: List<Row>,
    reactions: List<Row>,
    users: List<Row>,
    grouped: Boolean,
    wide: Boolean,
    initialEditing: Boolean = false,
    initialEditingText: String? = null,
    groupEnd: Boolean = true,
    canReact: Boolean = true,
    callbacks: MessageCallbacks? = null,
    ownOverride: Boolean? = null,
    canOpenThread: Boolean = true,
) {
    val id = message.text("id")
    val user =
        JsonObject(
            users.find { it.text("id") == message.text("user_id") }.orEmpty() +
                message.filterKeys {
                    it in setOf("display_name", "user_name", "avatar_url", "is_bot")
                }
        )
    val mine = ownOverride ?: (message.text("user_id") == app.config.userId)
    val editable = app.env.online && !channel.present("archive_at")
    var editing by remember(id, initialEditing) { mutableStateOf(initialEditing) }
    var content by
        remember(message.text("content"), initialEditingText) {
            mutableStateOf(initialEditingText ?: message.text("content"))
        }
    var deleting by remember(id) { mutableStateOf(false) }
    var picker by remember(id) { mutableStateOf(false) }
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    var focused by remember { mutableStateOf(false) }
    var menuOpen by remember(id) { mutableStateOf(false) }
    val gestureArbitration = remember(id) { MessageGestureArbitration() }
    val viewConfiguration = LocalViewConfiguration.current
    val selectionConfiguration =
        remember(viewConfiguration, gestureArbitration) {
            gestureArbitration.selectionConfiguration(viewConfiguration)
        }
    val openThread = {
        if (callbacks?.openThread != null) callbacks.openThread.invoke()
        else app.openThread(id, wide)
    }
    val startEdit = {
        if (callbacks?.startEdit != null) callbacks.startEdit.invoke() else editing = true
    }
    val requestDelete = {
        if (callbacks?.requestDelete != null) callbacks.requestDelete.invoke() else deleting = true
    }
    val pickReaction = {
        if (callbacks?.pickReaction != null) callbacks.pickReaction.invoke() else picker = true
    }
    val showThread =
        canOpenThread &&
            message.text("parent_id").isEmpty() &&
            (callbacks == null || callbacks.openThread != null)
    val canDelete =
        mine ||
            (callbacks == null &&
                !channel.flag("is_dm") &&
                canManage(channel, app.config.userId, app.config.isAdmin))
    CompositionLocalProvider(LocalViewConfiguration provides selectionConfiguration) {
        Box(
            Modifier.messageHold(!editing, gestureArbitration) { menuOpen = true }
                .semantics { testTag = "message:$id" }
                .fillMaxWidth()
                .background(
                    if (hovered || focused)
                        MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .6f)
                    else Color.Transparent,
                    RoundedCornerShape(8.dp),
                )
                .hoverable(interaction)
        ) {
            Row(
                Modifier.fillMaxWidth()
                    .padding(
                        start = 12.dp,
                        end = 12.dp,
                        top = if (grouped) 0.dp else 6.dp,
                        bottom = if (groupEnd) 6.dp else 0.dp,
                    ),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (grouped)
                    Box(
                        Modifier.width(32.dp).height(22.75.dp).graphicsLayer {
                            alpha = if (hovered || focused || menuOpen) 1f else 0f
                        },
                        contentAlignment = Alignment.CenterEnd,
                    ) {
                        Hint(app.platform.fullTimestamp(message.text("created_at"))) { hintModifier
                            ->
                            Text(
                                app.platform.time(message.text("created_at")),
                                hintModifier,
                                size = 10.sp,
                                muted = true,
                            )
                        }
                    }
                else Avatar(user, Modifier.padding(top = 2.dp))
                Column(Modifier.weight(1f)) {
                    if (!grouped)
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(6.dp),
                            ) {
                                val identity =
                                    if (message.flag("is_bot") && message.present("app_id"))
                                        Modifier.then(webControlModifier("button")).clickable(
                                            role = Role.Button
                                        ) {
                                            app.openApp(message.text("app_id"))
                                        }
                                    else Modifier
                                Text(
                                    author(user).ifEmpty { message.text("user_id") },
                                    identity,
                                    weight = FontWeight.SemiBold,
                                )
                                if (message.flag("is_bot"))
                                    Text(
                                        "APP",
                                        Modifier.background(
                                                MaterialTheme.colorScheme.surfaceVariant,
                                                RoundedCornerShape(4.dp),
                                            )
                                            .padding(horizontal = 4.dp, vertical = 2.dp),
                                        size = 9.sp,
                                        weight = FontWeight.SemiBold,
                                        muted = true,
                                    )
                            }
                            Hint(app.platform.fullTimestamp(message.text("created_at"))) {
                                hintModifier ->
                                Text(
                                    app.platform.time(message.text("created_at")),
                                    hintModifier,
                                    small = true,
                                    muted = true,
                                    size = 11.sp,
                                )
                            }
                        }
                    if (editing) {
                        Column(
                            Modifier.padding(top = 4.dp)
                                .border(
                                    1.dp,
                                    MaterialTheme.colorScheme.outline,
                                    RoundedCornerShape(8.dp),
                                )
                                .background(
                                    MaterialTheme.colorScheme.background,
                                    RoundedCornerShape(8.dp),
                                )
                                .padding(8.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Field(
                                content,
                                app.t("messages.editLabel"),
                                modifier =
                                    Modifier.onPreviewKeyEvent {
                                        if (
                                            it.type == KeyEventType.KeyDown && it.key == Key.Escape
                                        ) {
                                            if (callbacks?.cancelEdit != null)
                                                callbacks.cancelEdit.invoke()
                                            else editing = false
                                            true
                                        } else false
                                    },
                                singleLine = false,
                                showLabel = false,
                                autoFocus = true,
                                minimumHeight = 36.dp,
                            ) {
                                if (callbacks?.changeText != null) callbacks.changeText.invoke(it)
                                else content = it
                            }
                            Row(
                                Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                            ) {
                                Button(
                                    app.t("common.cancelShort"),
                                    size = "sm",
                                    variant = "ghost",
                                ) {
                                    if (callbacks?.cancelEdit != null) callbacks.cancelEdit.invoke()
                                    else editing = false
                                }
                                Button(
                                    app.t("common.save"),
                                    content.isNotBlank() && editable && "edit:$id" !in app.pending,
                                    primary = true,
                                    size = "sm",
                                ) {
                                    if (callbacks?.saveEdit != null) callbacks.saveEdit.invoke()
                                    else
                                        app.run("edit:$id") {
                                            val row =
                                                JsonObject(
                                                    message + ("content" to JsonPrimitive(content))
                                                )
                                            app.repo
                                                .messages(channel.text("id"))
                                                .write(
                                                    app.api,
                                                    "/api/messages/$id",
                                                    "PATCH",
                                                    obj("content" to content),
                                                    row,
                                                )
                                            editing = false
                                        }
                                }
                            }
                        }
                    } else {
                        val cards = messageCards(message["card"])
                        val blockCards =
                            parseBlocks(message["app_blocks"]).any { it.text("type") == "card" }
                        if (!blockCards) {
                            if (cards != null) cards.forEach { MessageCard(it) }
                            else {
                                if (message.text("content").isNotEmpty()) {
                                    CompositionLocalProvider(
                                        LocalMfmStyle provides
                                            androidx.compose.ui.text.TextStyle(
                                                lineHeight = 22.75.sp
                                            )
                                    ) {
                                        RichText(
                                            message.text("content"),
                                            users,
                                            editedAt =
                                                message.text("updated_at").takeIf {
                                                    it.isNotEmpty()
                                                },
                                        )
                                    }
                                    LinkPreviews(app, message.text("content"))
                                }
                            }
                        }
                        MessageAppContent(app, message)
                        MessageAttachments(
                            app,
                            LocalAttachments.current
                                .filter { it.text("message_id") == id }
                                .sortedBy { it.text("created_at") },
                        )
                    }
                    if (!editing) {
                        val replies =
                            all.count { it.text("parent_id") == id && !it.present("deleted_at") }
                        if (showThread && replies > 0)
                            Row(
                                Modifier.padding(top = 4.dp)
                                    .height(22.dp)
                                    .clip(RoundedCornerShape(12.dp))
                                    .border(
                                        1.dp,
                                        MaterialTheme.colorScheme.secondary.copy(alpha = .3f),
                                        RoundedCornerShape(12.dp),
                                    )
                                    .background(
                                        MaterialTheme.colorScheme.secondaryContainer.copy(
                                            alpha = .5f
                                        )
                                    )
                                    .clickable(role = Role.Button) {
                                        if (callbacks?.openThread != null)
                                            callbacks.openThread.invoke()
                                        else app.openThread(id, wide)
                                    }
                                    .semantics {
                                        contentDescription =
                                            app.t(
                                                "threads.openWithCount",
                                                "count" to replies.toString(),
                                            )
                                    }
                                    .padding(horizontal = 8.dp, vertical = 2.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                Icon(
                                    "message-square-text",
                                    modifier = Modifier.size(14.dp),
                                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                                )
                                Text(
                                    app.t("threads.replyCount", "count" to replies.toString()),
                                    small = true,
                                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                                )
                            }
                        val groups =
                            reactions
                                .filter { it.text("message_id") == id }
                                .groupBy { reactionKey(it.text("emoji")) }
                        if (groups.isNotEmpty())
                            FlowRow(
                                Modifier.padding(vertical = 4.dp)
                                    .then(
                                        webControlModifier(
                                            "group",
                                            attributes =
                                                mapOf("aria-label" to app.t("reactions.groupLabel")),
                                        )
                                    ),
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                                verticalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                groups.forEach { (emoji, rows) ->
                                    ReactionChip(
                                        app,
                                        emoji,
                                        rows,
                                        users,
                                        editable && canReact,
                                        wide,
                                    ) {
                                        if (callbacks?.reaction != null)
                                            callbacks.reaction.invoke(emoji)
                                        else
                                            toggleReaction(app, channel.text("id"), id, emoji, rows)
                                    }
                                }
                                IconButton(
                                    "smile-plus",
                                    app.t("reactions.add"),
                                    editable && canReact,
                                    size = 24.dp,
                                    click = pickReaction,
                                )
                            }
                    }
                }
            }
            if (!editing)
                Row(
                    Modifier.align(Alignment.TopEnd)
                        .padding(end = 6.dp)
                        .graphicsLayer { alpha = if (hovered || focused || menuOpen) 1f else 0f }
                        .onFocusChanged { focused = it.hasFocus }
                        .focusGroup()
                        .background(MaterialTheme.colorScheme.background, RoundedCornerShape(8.dp))
                        .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
                        .padding(2.dp),
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    if (!app.env.touch) {
                        IconButton(
                            "smile-plus",
                            app.t("reactions.add"),
                            editable && canReact,
                            size = 24.dp,
                            click = pickReaction,
                        )
                        if (showThread)
                            IconButton(
                                "message-square-plus",
                                app.t("threads.open"),
                                size = 24.dp,
                                click = openThread,
                            )
                        if (mine)
                            IconButton(
                                "pencil",
                                app.t("messages.edit"),
                                size = 24.dp,
                                click = startEdit,
                            )
                        if (canDelete)
                            IconButton(
                                "trash-2",
                                app.t("messages.delete"),
                                size = 24.dp,
                                click = requestDelete,
                            )
                    }
                    Menu(
                        app.t("messages.more"),
                        buildList {
                            add(
                                MenuAction("copy", app.t("messages.copyRaw")) {
                                    app.run {
                                        try {
                                            app.platform.invoke("copy", message.text("content"))
                                            app.showToast(
                                                app.t("messages.copied"),
                                                ToastKind.Success,
                                            )
                                        } catch (e: CancellationException) {
                                            throw e
                                        } catch (_: Exception) {
                                            app.toast = app.t("messages.copyFailed")
                                        }
                                    }
                                }
                            )
                            add(
                                MenuAction("bot", app.t("apps.messageActions")) {
                                    app.target = id
                                    app.dialog = "apps"
                                }
                            )
                            add(
                                MenuAction(
                                    "smile-plus",
                                    app.t("reactions.add"),
                                    editable && canReact,
                                    pickReaction,
                                )
                            )
                            if (showThread)
                                add(
                                    MenuAction(
                                        "message-square-plus",
                                        app.t("threads.open"),
                                        action = openThread,
                                    )
                                )
                            if (mine)
                                add(
                                    MenuAction("pencil", app.t("messages.edit"), action = startEdit)
                                )
                            if (canDelete)
                                add(
                                    MenuAction(
                                        "trash-2",
                                        app.t("messages.delete"),
                                        action = requestDelete,
                                    )
                                )
                        },
                        size = 24.dp,
                        controlledOpen = menuOpen,
                        onOpenChange = { menuOpen = it },
                    )
                }
        }
    }
    if (deleting)
        Confirm(
            app.t("messages.deleteTitle"),
            app.t("messages.deleteDescription"),
            app.t("messages.delete"),
            "delete:$id" in app.pending,
            { deleting = false },
        ) {
            app.run("delete:$id") {
                app.repo
                    .messages(channel.text("id"))
                    .write(
                        app.api,
                        "/api/messages/$id",
                        "DELETE",
                        null,
                        JsonObject(message + ("deleted_at" to JsonPrimitive(app.platform.now()))),
                    )
                deleting = false
            }
        }
    if (picker)
        EmojiPicker(app, { picker = false }) { emoji ->
            picker = false
            toggleReaction(
                app,
                channel.text("id"),
                id,
                emoji,
                reactions.filter {
                    it.text("message_id") == id &&
                        reactionKey(it.text("emoji")) == reactionKey(emoji)
                },
            )
        }
}

private fun toggleReaction(
    app: Session,
    channel: String,
    message: String,
    emoji: String,
    rows: List<Row>,
) {
    val mine = rows.find { it.text("user_id") == app.config.userId }
    app.run("reaction:$message:$emoji") {
        val row =
            mine
                ?: obj(
                    "id" to app.platform.uuid(),
                    "message_id" to message,
                    "user_id" to app.config.userId,
                    "emoji" to emoji,
                    "created_at" to app.platform.now(),
                )
        app.repo
            .reactions(channel)
            .write(
                app.api,
                "/api/messages/$message/reactions" +
                    if (mine != null) "?emoji=${mine.text("emoji").encodeURLParameter()}" else "",
                if (mine != null) "DELETE" else "POST",
                if (mine != null) null else row,
                if (mine != null) null else row,
                key = rowKey(row),
            )
    }
}

@Composable
fun TypingStatus(app: Session, names: List<String>) {
    Box(
        Modifier.fillMaxWidth()
            .heightIn(min = 24.dp)
            .padding(horizontal = 16.dp)
            .then(
                webControlModifier(
                    "status",
                    attributes = mapOf("aria-live" to "polite", "aria-atomic" to "true"),
                )
            )
    ) {
        if (names.isNotEmpty())
            Text(
                app.t(
                    "messages.typing",
                    "names" to names.joinToString(app.t("messages.typingSeparator")),
                ),
                small = true,
                muted = true,
            )
    }
}

@Composable
internal fun Composer(
    app: Session,
    channel: String,
    parent: String?,
    users: List<Row>,
    enabled: Boolean,
    initialValue: String = "",
    sendMessage: (suspend (String, String?, List<String>) -> Boolean)? = null,
    uploadAttachment: (suspend (Row) -> Row)? = null,
    allowAttachments: Boolean = true,
    showTyping: Boolean = true,
    placeholder: String? = null,
    ariaLabel: String? = null,
    commandOptions: List<Row>? = null,
    mentionMemberIds: Set<String>? = null,
    onTyping: ((Boolean) -> Unit)? = null,
    onRequestCommands: (() -> Unit)? = null,
) {
    val key = "draft:$channel:${parent.orEmpty()}"
    var value by remember(key, initialValue) { mutableStateOf(initialValue) }
    var picker by remember(key) { mutableStateOf(false) }
    var composing by remember(key) { mutableStateOf(false) }
    var caret by remember(key) { mutableStateOf(0) }
    var selection by remember(key) { mutableStateOf<Int?>(null) }
    var activeOption by remember(key) { mutableStateOf(0) }
    var dismissed by remember(key) { mutableStateOf(false) }
    var typingActive by remember(key) { mutableStateOf(false) }
    val composerFocusRequester = remember { FocusRequester() }
    var focusReturn by remember(key) { mutableIntStateOf(0) }
    fun updateTyping(active: Boolean) {
        typingActive = active
        onTyping?.invoke(active)
    }
    LaunchedEffect(focusReturn) {
        if (focusReturn > 0 && enabled) {
            withFrameNanos {}
            composerFocusRequester.requestFocus()
        }
    }
    val memberships by app.repo.members.rows.collectAsState()
    val channels by app.repo.channels.rows.collectAsState()
    var commands by remember(channel) { mutableStateOf<List<Row>>(emptyList()) }
    val attachments = rememberAttachmentDrafts(app, channel, key, enabled, uploadAttachment)
    attachments.acceptFiles = allowAttachments
    val density = LocalDensity.current.density
    var sendError by remember(key) { mutableStateOf<String?>(null) }
    var selectedCommand by remember(key) { mutableStateOf<CompletionOption?>(null) }
    LaunchedEffect(
        channel,
        app.env.online,
        enabled,
        value.startsWith("/"),
        commandOptions,
        onRequestCommands,
    ) {
        if (app.env.online && enabled && value.startsWith("/"))
            try {
                if (onRequestCommands != null) onRequestCommands()
                else if (commandOptions == null)
                    commands =
                        app.api
                            .request(
                                "/api/apps/commands?channel_id=${channel.encodeURLParameter()}"
                            )
                            .rows()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {}
    }
    val range = if (enabled && !dismissed && !composing) completionAt(value, caret) else null
    val options =
        completionOptions(
            range,
            users,
            mentionMemberIds
                ?: memberships
                    .filter { it.text("channel_id") == channel }
                    .map { it.text("user_id") }
                    .toSet(),
            channels,
            commandOptions ?: commands,
        )
    fun choose(option: CompletionOption) {
        if (range != null) {
            if (range.kind == "command") selectedCommand = option
            val (next, position) = insertCompletion(value, range, option.name)
            value = next
            caret = position
            selection = position
            dismissed = true
            updateTyping(true)
            focusReturn++
        }
    }

    var typing by remember(key) { mutableStateOf<List<String>>(emptyList()) }
    val typingSession = remember { app.platform.uuid() }
    LaunchedEffect(key) {
        val stored = app.platform.invoke("storageRead", key)
        if (value.isEmpty()) {
            value = stored
            caret = stored.length
        }
    }
    LaunchedEffect(value, key) {
        delay(250)
        app.platform.invoke("storageWrite", obj("key" to key, "value" to value).toString())
    }
    LaunchedEffect(typingActive, enabled, app.env.visible, onTyping) {
        if (onTyping == null && typingActive && enabled && app.env.visible) {
            try {
                while (isActive) {
                    try {
                        app.api.request(
                            "/api/channels/$channel/typing",
                            "POST",
                            buildJsonObject {
                                put("parent_id", parent?.let(::JsonPrimitive) ?: JsonNull)
                                put("session_id", typingSession)
                                put("active", true)
                            },
                        )
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {}
                    delay(2500)
                }
            } finally {
                withContext(NonCancellable) {
                    try {
                        app.api.request(
                            "/api/channels/$channel/typing",
                            "POST",
                            buildJsonObject {
                                put("parent_id", parent?.let(::JsonPrimitive) ?: JsonNull)
                                put("session_id", typingSession)
                                put("active", false)
                            },
                        )
                    } catch (_: Exception) {}
                }
            }
        }
    }
    LaunchedEffect(key, app.env.visible, enabled) {
        while (enabled && app.env.visible && isActive) {
            try {
                typing =
                    app.api
                        .request(
                            "/api/channels/$channel/typing" +
                                (parent?.let { "?parent_id=$it" } ?: "")
                        )
                        .rows()
                        .filter { it.text("user_id") != app.config.userId }
                        .map { row ->
                            users.find { it.text("id") == row.text("user_id") }?.let(::author)
                                ?: row.text("user_id")
                        }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {}
            delay(1500)
        }
    }
    val send = {
        if (attachments.canSend(value) && key !in app.pending && !composing) {
            val sent = value
            val commandAppId = selectedCommand?.appId
            updateTyping(false)
            val drafts = attachments.items.toList()
            val attachmentIds = drafts.mapNotNull { it.uploaded?.text("id") }
            attachments.items.clear()
            attachments.error = null
            sendError = null
            value = ""
            caret = 0
            app.run(key) {
                val successful =
                    try {
                        sendMessage?.invoke(sent.trim(), commandAppId, attachmentIds)
                            ?: app.send(channel, sent.trim(), parent, commandAppId, attachmentIds)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        false
                    }
                if (!successful) {
                    sendError =
                        if (sendMessage != null) null else app.toast ?: app.t("common.writeError")
                    app.toast = null
                    attachments.items.addAll(drafts)
                    if (value.isEmpty()) {
                        value = sent
                        caret = sent.length
                    }
                } else attachments.release(drafts)
            }
        }
        Unit
    }
    if (showTyping) TypingStatus(app, typing)
    val composerBorder = MaterialTheme.colorScheme.outline
    Column(
        Modifier.fillMaxWidth()
            .drawBehind {
                drawLine(
                    composerBorder,
                    androidx.compose.ui.geometry.Offset(0f, .5.dp.toPx()),
                    androidx.compose.ui.geometry.Offset(size.width, .5.dp.toPx()),
                    1.dp.toPx(),
                )
            }
            .padding(top = 1.dp)
            .padding(horizontal = 12.dp, vertical = 12.dp)
    ) {
        sendError?.let { error ->
            Row(
                Modifier.fillMaxWidth().padding(bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    error,
                    Modifier.weight(1f).semantics { liveRegion = LiveRegionMode.Assertive },
                    color = MaterialTheme.colorScheme.error,
                    small = true,
                )
                IconButton("x", app.t("common.close"), size = 24.dp) { sendError = null }
            }
        }
        DraftAttachments(app, attachments)
        if (options.isNotEmpty())
            Column(
                Modifier.fillMaxWidth()
                    .padding(bottom = 8.dp)
                    .heightIn(max = 192.dp)
                    .verticalScroll(rememberScrollState())
                    .then(
                        webControlModifier(
                            "listbox",
                            attributes =
                                mapOf(
                                    "aria-label" to
                                        app.t(
                                            when (range?.kind) {
                                                "command" -> "composer.commands"
                                                "channel" -> "composer.channels"
                                                "mfm" -> "mfm.suggestions"
                                                else -> "mentions.suggestions"
                                            }
                                        )
                                ),
                        )
                    )
                    .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
                    .padding(4.dp)
            ) {
                options.forEachIndexed { index, option ->
                    val optionName =
                        when (range?.kind) {
                            "mfm" -> "$[${option.name}]"
                            "channel" -> "#${option.name}"
                            "command" -> "/${option.name}"
                            else -> "@${option.name}"
                        }
                    val optionLabel = if (range?.mfm == true) app.t(option.label) else option.label
                    Row(
                        Modifier.fillMaxWidth()
                            .then(
                                webControlModifier(
                                    "option",
                                    selected = index == activeOption,
                                    attributes =
                                        mapOf(
                                            "aria-label" to
                                                if (optionLabel.isBlank()) optionName
                                                else "$optionName $optionLabel"
                                        ),
                                )
                            )
                            .background(
                                if (index == activeOption) MaterialTheme.colorScheme.surfaceVariant
                                else Color.Transparent,
                                RoundedCornerShape(4.dp),
                            )
                            .clickable { choose(option) }
                            .padding(horizontal = 8.dp, vertical = 4.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        androidx.compose.material3.Text(
                            fontText(
                                optionName,
                                LocalFonts.current.copy(latin = LocalFonts.current.mono),
                            ),
                            style =
                                MaterialTheme.typography.bodyMedium.copy(
                                    fontFamily = LocalFonts.current.mono
                                ),
                            color = MaterialTheme.colorScheme.onBackground,
                        )
                        if (optionLabel.isNotBlank()) Text(optionLabel, muted = true, maxLines = 1)
                    }
                }
            }
        ComposerFrame(Modifier.fillMaxWidth()) {
            if (allowAttachments)
                IconButton(
                    "paperclip",
                    app.t("attachments.add"),
                    enabled && attachments.items.size < MAX_ATTACHMENTS,
                    color = MaterialTheme.colorScheme.onBackground,
                    size = if (app.env.touch) 44.dp else 36.dp,
                ) {
                    attachments.pick()
                }
            Field(
                value,
                ariaLabel
                    ?: placeholder
                    ?: app.t(if (parent == null) "messages.compose" else "threads.compose"),
                Modifier.weight(1f)
                    .onGloballyPositioned { coordinates ->
                        val bounds = coordinates.boundsInWindow()
                        attachments.updateBounds(
                            bounds.left / density,
                            bounds.top / density,
                            bounds.width / density,
                            bounds.height / density,
                        )
                    }
                    .onPreviewKeyEvent {
                        if (composing || it.type != KeyEventType.KeyDown) false
                        else if (
                            options.isNotEmpty() &&
                                !it.isCtrlPressed &&
                                !it.isMetaPressed &&
                                !it.isShiftPressed
                        )
                            when (it.key) {
                                Key.DirectionDown -> {
                                    activeOption = (activeOption + 1) % options.size
                                    true
                                }
                                Key.DirectionUp -> {
                                    activeOption = (activeOption + options.size - 1) % options.size
                                    true
                                }
                                Key.Tab,
                                Key.Enter -> {
                                    choose(options[activeOption.coerceIn(0, options.lastIndex)])
                                    true
                                }
                                Key.Escape -> {
                                    dismissed = true
                                    true
                                }
                                else -> false
                            }
                        else if (
                            !it.isShiftPressed &&
                                it.key == Key.Enter &&
                                (if (app.env.mac) it.isMetaPressed else it.isCtrlPressed)
                        ) {
                            send()
                            true
                        } else false
                    },
                singleLine = false,
                enabled = enabled,
                readOnly = !enabled,
                placeholder =
                    placeholder
                        ?: app.t(if (parent == null) "messages.compose" else "threads.compose"),
                focusRequester = composerFocusRequester,
                onBlur = {
                    dismissed = true
                    updateTyping(false)
                },
                onComposition = { composing = it },
                fileOwner = attachments.owner.takeIf { allowAttachments },
                chrome = false,
                selection = selection,
                onSelection = {
                    caret = it
                    selection = null
                },
            ) {
                value = it
                updateTyping(it.trim().isNotEmpty())
                if (selectedCommand?.let { command -> !it.startsWith("/${command.name} ") } == true)
                    selectedCommand = null
                dismissed = false
                activeOption = 0
            }
            val canSubmit = attachments.canSend(value) && key !in app.pending && !composing
            Box(
                Modifier.padding(bottom = 4.dp)
                    .then(webControlModifier("button", canSubmit))
                    .size(if (app.env.touch) 44.dp else 36.dp)
                    .padding(1.dp)
                    .clip(RoundedCornerShape(9.dp))
                    .background(
                        if (canSubmit) MaterialTheme.colorScheme.primary
                        else MaterialTheme.colorScheme.surfaceVariant
                    )
                    .clickable(canSubmit, role = Role.Button, onClick = send),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    "send",
                    app.t("messages.send"),
                    modifier = Modifier.size(16.dp),
                    color =
                        if (canSubmit) MaterialTheme.colorScheme.onPrimary
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        if (!app.env.touch)
            Text(
                app.t(if (app.env.mac) "composer.mac" else "composer.ctrl"),
                Modifier.padding(top = 6.dp, start = 4.dp),
                muted = true,
                small = true,
                size = 11.sp,
            )
    }
    if (picker)
        EmojiPicker(app, { picker = false }) {
            value += it
            picker = false
        }
}

@Composable
internal fun MessageLoadingSkeleton() {
    val color = MaterialTheme.colorScheme.surfaceVariant
    Column(
        Modifier.fillMaxWidth().padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        repeat(6) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Box(Modifier.size(36.dp).background(color, CircleShape))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.size(112.dp, 12.dp).background(color, RoundedCornerShape(8.dp)))
                    Box(
                        Modifier.widthIn(max = 448.dp)
                            .fillMaxWidth()
                            .height(16.dp)
                            .background(color, RoundedCornerShape(8.dp))
                    )
                }
            }
        }
    }
}

@Composable
internal fun MessageEmptyState(
    app: Session,
    title: String,
    description: String?,
    destructive: Boolean = false,
    retry: (() -> Unit)? = null,
) {
    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                app.t(title),
                weight = FontWeight.Medium,
                color = if (destructive) MaterialTheme.colorScheme.error else Color.Unspecified,
            )
            if (description != null) Text(app.t(description), small = true, muted = true)
            if (retry != null)
                Button(app.t("offline.retry"), variant = "outline", size = "sm", click = retry)
        }
    }
}
