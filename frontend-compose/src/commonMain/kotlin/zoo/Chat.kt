package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.*
import androidx.compose.ui.graphics.*
import androidx.compose.ui.input.key.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import kotlinx.serialization.json.*
import org.jetbrains.compose.resources.decodeToImageBitmap

@Composable
fun Avatar(user: Row?, modifier: Modifier = Modifier) {
    val name = user?.let(::author).orEmpty().ifEmpty { "?" }
    BoxWithConstraints(
        modifier
            .size(32.dp)
            .clip(CircleShape)
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .border(1.dp, MaterialTheme.colorScheme.outline, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        val url = user?.text("avatar_url").orEmpty()
        if (url.isNotEmpty()) RemoteImage(url, name, Modifier.fillMaxSize())
        else if (user?.flag("is_bot") == true) Icon("bot", modifier = Modifier.size(16.dp))
        else
            Text(
                name
                    .trim()
                    .split(Regex("\\s+"))
                    .take(2)
                    .joinToString("") { it.take(1) }
                    .uppercase(),
                small = true,
                size = if (maxWidth > 24.dp) 14.sp else 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
    }
}

@Composable
fun RemoteImage(url: String, label: String, modifier: Modifier = Modifier) {
    val app = LocalApp.current
    var bitmap by remember(url) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(url) {
        try {
            if (
                url.startsWith("https://") ||
                    url.startsWith("http://") ||
                    (url.startsWith("/") && !url.startsWith("//"))
            )
                bitmap = app.image(url).decodeToImageBitmap()
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {}
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
) {
    val id = channel.text("id")
    val table = remember(id) { app.repo.messages(id) }
    DisposableEffect(id) { onDispose { app.repo.releaseChannel(id) } }
    val all by table.rows.collectAsState()
    val loading by table.loading.collectAsState()
    val error by table.error.collectAsState()
    val reactions by app.repo.reactions(id).rows.collectAsState()
    val joined = isMember(members, id, app.config.userId)
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
    Row(modifier.fillMaxWidth()) {
        if (wide || app.thread == null)
            Column(Modifier.weight(1f).fillMaxHeight()) {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(app.t("messages.title"), weight = FontWeight.SemiBold)
                    Text(
                        if (loading) "…"
                        else
                            app.t(
                                "messages.count",
                                "count" to visibleMessages(all).size.toString(),
                            ),
                        Modifier.background(
                                MaterialTheme.colorScheme.surfaceVariant,
                                RoundedCornerShape(3.dp),
                            )
                            .padding(horizontal = 8.dp, vertical = 2.dp),
                        small = true,
                        muted = true,
                    )
                }
                HorizontalDivider()
                MessageList(app, channel, all, reactions, users, null, wide, Modifier.weight(1f))
                if (error != null) {
                    Text(app.t("messages.loadError"), Modifier.padding(12.dp))
                    Button(app.t("offline.retry")) { table.refresh() }
                }
                if (loading && all.isEmpty())
                    Text(app.t("common.loading"), Modifier.padding(16.dp), muted = true)
                when {
                    channel.present("archive_at") ->
                        Text(
                            app.t("moderation.archivedReadOnly"),
                            Modifier.padding(16.dp),
                            muted = true,
                        )
                    !joined ->
                        Row(
                            Modifier.padding(16.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                app.t(
                                    if (channel.flag("is_private")) "channels.privateMessages"
                                    else "messages.joinToPostDescription"
                                ),
                                Modifier.weight(1f),
                                muted = true,
                            )
                            if (!channel.flag("is_private"))
                                Button(
                                    app.t("messages.joinToPost"),
                                    app.env.online,
                                    primary = true,
                                ) {
                                    app.run("join:$id") {
                                        val row =
                                            obj("channel_id" to id, "user_id" to app.config.userId)
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
                    else -> Composer(app, id, null, users, canPost)
                }
            }
        app.thread?.let { parent ->
            if (wide) VerticalDivider()
            Column(
                if (wide) Modifier.width(384.dp).fillMaxHeight()
                else Modifier.weight(1f).fillMaxHeight()
            ) {
                Row(
                    Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(app.t("threads.title"), Modifier.weight(1f), weight = FontWeight.SemiBold)
                    IconButton("x", app.t("threads.close")) { app.thread = null }
                }
                HorizontalDivider()
                all.find { it.text("id") == parent && !it.present("deleted_at") }
                    ?.let { Message(app, channel, it, all, reactions, users, false, wide) }
                MessageList(app, channel, all, reactions, users, parent, wide, Modifier.weight(1f))
                if (joined && !channel.present("archive_at"))
                    Composer(app, id, parent, users, canPost)
            }
        }
    }
}

@Composable
private fun MessageList(
    app: Session,
    channel: Row,
    all: List<Row>,
    reactions: List<Row>,
    users: List<Row>,
    parent: String?,
    wide: Boolean,
    modifier: Modifier,
) {
    val messages = visibleMessages(all, parent)
    val state = rememberLazyListState()
    LaunchedEffect(app.query["message"], messages.size) {
        val index = messages.indexOfFirst { it.text("id") == app.query["message"] }
        if (index >= 0) state.scrollToItem(index * 2 + 1)
    }
    var previousSize by remember(channel.text("id"), parent) { mutableStateOf(0) }
    LaunchedEffect(messages.size) {
        if (
            messages.isNotEmpty() &&
                (previousSize == 0 ||
                    state.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 >=
                        previousSize - 2 ||
                    messages.last().text("user_id") == app.config.userId)
        )
            state.scrollToItem(messages.lastIndex * 2)
        previousSize = messages.size
    }
    LaunchedEffect(channel.text("id"), parent, app.env.visible, app.env.online, all.size) {
        if (parent == null && app.env.visible && app.env.online) {
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
    LazyColumn(
        modifier.fillMaxWidth(),
        state = state,
        contentPadding = PaddingValues(vertical = 12.dp),
    ) {
        if (messages.isEmpty())
            item {
                Box(Modifier.fillParentMaxSize(), contentAlignment = Alignment.Center) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(app.t("messages.empty"), muted = true)
                        Text(app.t("messages.first"), small = true, muted = true)
                    }
                }
            }
        messages.forEachIndexed { index, row ->
            val previous = messages.getOrNull(index - 1)
            item("date:${row.text("id")}") {
                if (
                    previous == null ||
                        app.platform.day(previous.text("created_at")) !=
                            app.platform.day(row.text("created_at"))
                )
                    Row(
                        Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        HorizontalDivider(Modifier.weight(1f))
                        Box(
                            Modifier.height(21.dp)
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
                Message(app, channel, row, all, reactions, users, groupedWith(previous, row), wide)
            }
        }
    }
}

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
) {
    val id = message.text("id")
    val user = users.find { it.text("id") == message.text("user_id") }
    val mine = message.text("user_id") == app.config.userId
    val editable = app.env.online && !channel.present("archive_at")
    var editing by remember(id) { mutableStateOf(false) }
    var content by remember(message.text("content")) { mutableStateOf(message.text("content")) }
    var deleting by remember(id) { mutableStateOf(false) }
    var picker by remember(id) { mutableStateOf(false) }
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    var focused by remember { mutableStateOf(false) }
    Box(Modifier.semantics { testTag = "message:$id" }.fillMaxWidth().hoverable(interaction)) {
        Row(
            Modifier.fillMaxWidth()
                .padding(horizontal = 20.dp, vertical = if (grouped) 0.dp else 6.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (grouped) Spacer(Modifier.width(32.dp))
            else Avatar(user, Modifier.padding(top = 2.dp))
            Column(Modifier.weight(1f)) {
                if (!grouped)
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(
                            user?.let(::author) ?: message.text("user_id"),
                            weight = FontWeight.SemiBold,
                        )
                        Text(
                            app.platform.time(message.text("created_at")),
                            small = true,
                            muted = true,
                            size = 11.sp,
                        )
                        if (message.present("app_id")) Text("APP", small = true, muted = true)
                    }
                if (editing) {
                    Field(content, app.t("messages.editLabel"), singleLine = false) { content = it }
                    Row {
                        Button(app.t("common.cancelShort")) { editing = false }
                        Button(
                            app.t("common.save"),
                            content.isNotBlank() && editable && "edit:$id" !in app.pending,
                            primary = true,
                        ) {
                            app.run("edit:$id") {
                                val row =
                                    JsonObject(message + ("content" to JsonPrimitive(content)))
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
                } else {
                    RichText(message.text("content"), users)
                    if (message.present("card")) MessageCard(message.nested("card"))
                    MessageAppContent(app, message)
                }
                val groups =
                    reactions
                        .filter { it.text("message_id") == id }
                        .groupBy { reactionKey(it.text("emoji")) }
                Row(
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    groups.forEach { (emoji, rows) ->
                        Row(
                            Modifier.border(
                                    1.dp,
                                    MaterialTheme.colorScheme.outline,
                                    RoundedCornerShape(8.dp),
                                )
                                .clickable(enabled = editable) {
                                    toggleReaction(app, channel.text("id"), id, emoji, rows)
                                }
                                .padding(horizontal = 6.dp, vertical = 2.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            ReactionEmoji(emoji)
                            Text(
                                rows.map { it.text("user_id") }.distinct().size.toString(),
                                Modifier.padding(start = 4.dp),
                                small = true,
                            )
                        }
                    }
                    val replies = all.count {
                        it.text("parent_id") == id && !it.present("deleted_at")
                    }
                    if (message.text("parent_id").isEmpty() && replies > 0)
                        Row(
                            Modifier.height(22.dp)
                                .clip(RoundedCornerShape(12.dp))
                                .border(
                                    1.dp,
                                    MaterialTheme.colorScheme.secondary.copy(alpha = .3f),
                                    RoundedCornerShape(12.dp),
                                )
                                .background(
                                    MaterialTheme.colorScheme.secondaryContainer.copy(alpha = .5f)
                                )
                                .clickable(role = Role.Button) { app.openThread(id, wide) }
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
                }
            }
        }
        Row(
            Modifier.align(Alignment.TopEnd)
                .padding(end = 14.dp)
                .graphicsLayer { alpha = if (hovered || focused || app.env.touch) 1f else 0f }
                .onFocusChanged { focused = it.hasFocus }
                .focusGroup()
                .background(MaterialTheme.colorScheme.background)
                .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
        ) {
            IconButton("smile-plus", app.t("reactions.add"), editable, size = 24.dp) {
                picker = true
            }
            Menu(
                app.t("messages.more"),
                buildList {
                    if (message.text("parent_id").isEmpty())
                        add(
                            MenuAction("message-square-text", app.t("threads.open")) {
                                app.openThread(id, wide)
                            }
                        )
                    if (mine)
                        add(
                            MenuAction("pencil", app.t("messages.edit"), editable) {
                                editing = true
                            }
                        )
                    if (
                        mine ||
                            !channel.flag("is_dm") &&
                                canManage(channel, app.config.userId, app.config.isAdmin)
                    )
                        add(
                            MenuAction("trash-2", app.t("messages.delete"), editable) {
                                deleting = true
                            }
                        )
                    add(
                        MenuAction("copy", app.t("apps.copy")) {
                            app.run { app.platform.invoke("copy", message.text("content")) }
                        }
                    )
                    add(
                        MenuAction("bot", app.t("apps.messageActions"), app.env.online) {
                            app.target = id
                            app.dialog = "apps"
                        }
                    )
                },
            )
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
internal fun Composer(
    app: Session,
    channel: String,
    parent: String?,
    users: List<Row>,
    enabled: Boolean,
) {
    val key = "draft:$channel:${parent.orEmpty()}"
    var value by remember(key) { mutableStateOf("") }
    var picker by remember(key) { mutableStateOf(false) }
    var composing by remember(key) { mutableStateOf(false) }
    var caret by remember(key) { mutableStateOf(0) }
    var selection by remember(key) { mutableStateOf<Int?>(null) }
    var activeOption by remember(key) { mutableStateOf(0) }
    var dismissed by remember(key) { mutableStateOf(false) }
    val memberships by app.repo.members.rows.collectAsState()
    val range = if (enabled && !dismissed && !composing) completionAt(value, caret) else null
    val options =
        completionOptions(
            range,
            users,
            memberships
                .filter { it.text("channel_id") == channel }
                .map { it.text("user_id") }
                .toSet(),
        )
    fun choose(option: CompletionOption) {
        if (range != null) {
            val (next, position) = insertCompletion(value, range, option.name)
            value = next
            caret = position
            selection = position
            dismissed = true
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
    LaunchedEffect(value.isNotEmpty(), enabled, app.env.visible) {
        if (value.isNotEmpty() && enabled && app.env.visible) {
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
        if (value.isNotBlank() && enabled && key !in app.pending) {
            val sent = value
            value = ""
            caret = 0
            app.run(key) {
                if (!app.send(channel, sent, parent) && value.isEmpty()) {
                    value = sent
                    caret = sent.length
                }
            }
        }
        Unit
    }
    Column(
        Modifier.fillMaxWidth()
            .border(BorderStroke(1.dp, MaterialTheme.colorScheme.outline))
            .padding(horizontal = 12.dp, vertical = 12.dp)
    ) {
        if (typing.isNotEmpty())
            Text(
                app.t(
                    "messages.typing",
                    "names" to typing.joinToString(app.t("messages.typingSeparator")),
                ),
                small = true,
                muted = true,
            )
        if (options.isNotEmpty())
            Column(
                Modifier.fillMaxWidth()
                    .heightIn(max = 192.dp)
                    .verticalScroll(rememberScrollState())
                    .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
                    .padding(4.dp)
            ) {
                options.forEachIndexed { index, option ->
                    Button(
                        "${if (range?.mfm == true) "$[${option.name}]" else "@${option.name}"}   ${if (range?.mfm == true) app.t(option.label) else option.label}",
                        modifier =
                            Modifier.fillMaxWidth()
                                .background(
                                    if (index == activeOption)
                                        MaterialTheme.colorScheme.surfaceVariant
                                    else Color.Transparent
                                ),
                    ) {
                        choose(option)
                    }
                }
            }
        Row(
            Modifier.fillMaxWidth()
                .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(12.dp))
                .padding(6.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            Field(
                value,
                app.t(if (parent == null) "messages.compose" else "threads.compose"),
                Modifier.weight(1f).onPreviewKeyEvent {
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
                readOnly = !enabled,
                onComposition = { composing = it },
                chrome = false,
                selection = selection,
                onSelection = {
                    caret = it
                    selection = null
                },
            ) {
                value = it
                dismissed = false
                activeOption = 0
            }
            Box(
                Modifier.padding(bottom = 4.dp)
                    .size(if (app.env.touch) 44.dp else 36.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(
                        if (enabled && value.isNotBlank()) MaterialTheme.colorScheme.primary
                        else MaterialTheme.colorScheme.primary.copy(alpha = .5f)
                    )
                    .clickable(
                        enabled && value.isNotBlank() && key !in app.pending,
                        onClick = send,
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Icon("send", app.t("messages.send"), color = MaterialTheme.colorScheme.onPrimary)
            }
        }
        if (!app.env.touch)
            Text(
                app.t(if (app.env.mac) "composer.mac" else "composer.ctrl"),
                Modifier.padding(top = 8.dp, start = 4.dp),
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
