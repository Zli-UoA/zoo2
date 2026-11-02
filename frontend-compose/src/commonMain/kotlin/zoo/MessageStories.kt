package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.unit.*
import kotlinx.serialization.json.*

private fun Row.rows(key: String): List<Row> =
    (get(key) as? JsonArray)?.mapNotNull { it as? JsonObject }.orEmpty()

@Composable
fun MessageStory(app: Session, options: Row) {
    val fixture = options.nested("fixture")
    val args = fixture.nested("args")
    val component = fixture.text("component")
    val users = fixture.rows("users")
    val channel =
        args.nested("channel").ifEmpty {
            fixture.rows("channels").find { it.text("id") == args.text("channelId") }
                ?: CatalogSamples.channel
        }
    val messages = if ("messages" in args) args.rows("messages") else fixture.rows("messages")
    when (component) {
        "MessageContent" -> RichText(args.text("content"), users)
        "LinkPreviews" -> LinkPreviews(app, args.text("content"))
        "MessageCard" ->
            if (fixture.rows("feed").isNotEmpty())
                Column(
                    Modifier.widthIn(max = 672.dp).padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(28.dp),
                ) {
                    fixture.rows("feed").forEach { item ->
                        Column {
                            Text(item.text("body"))
                            MessageCard(item.nested("card"), true)
                        }
                    }
                }
            else MessageCard(args.nested("card"), args.flag("attachment"))
        "ReactionEmoji" -> {
            if (fixture.text("state") == "Consistent Size")
                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    ReactionEmoji("🎉", loadCatalog = false)
                    ReactionEmoji(
                        "kitchen:1f600_1f431",
                        catalog = args.nested("catalog").takeIf { it.isNotEmpty() },
                        loadCatalog = false,
                    )
                    ReactionEmoji("kitchen:1f600_1f431", loadCatalog = false)
                }
            else
                ReactionEmoji(
                    args.text("emoji"),
                    catalog = args.nested("catalog").takeIf { it.isNotEmpty() },
                    loadCatalog = false,
                    size = (args.text("size").toIntOrNull() ?: 20).dp,
                )
        }
        "ReactionPicker" -> {
            var open by remember { mutableStateOf(true) }
            Button(app.t("reactions.add"), primary = true) { open = true }
            if (open)
                EmojiPicker(
                    app,
                    { open = false },
                    kitchenCatalogUrl = fixture.text("kitchenCatalogUrl").ifEmpty { null },
                    initialRecents =
                        if (fixture.text("state") == "Recents") listOf("🎉", "👍") else null,
                ) {
                    app.run {
                        app.platform.invoke(
                            "storyCallback",
                            storyPayload("onSelect", listOf(JsonPrimitive(it))),
                        )
                    }
                    open = false
                }
        }
        "Typing" -> TypingStatus(app, args.rows("users").map(::author))
        "Composer" ->
            StoryComposer(
                app,
                fixture,
                args,
                users,
                channel.text("id").ifEmpty { "catalog" },
                null,
                "onSend",
            )
        "MessageRow" ->
            StoryMessage(app, fixture, args, args.nested("item").nested("message"), users, channel)
        "MessageList" -> StoryMessageList(app, fixture, args, messages, users, channel)
        "ThreadPane" ->
            Box(
                Modifier.fillMaxSize()
                    .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .3f)),
                contentAlignment = Alignment.CenterEnd,
            ) {
                ThreadSurface(
                    app,
                    args.rows("replies").size,
                    args.flag("isLoading"),
                    close = {
                        app.run {
                            app.platform.invoke(
                                "storyCallback",
                                storyPayload("onClose", emptyList()),
                            )
                        }
                    },
                    root = {
                        StoryMessage(app, fixture, args, args.nested("root"), users, channel)
                    },
                    replies = {
                        StoryMessageList(app, fixture, args, args.rows("replies"), users, channel)
                    },
                    composer = {
                        StoryComposer(
                            app,
                            fixture,
                            args,
                            users,
                            channel.text("id").ifEmpty { "catalog" },
                            args.nested("root").text("id"),
                            "onSendReply",
                        )
                    },
                )
            }
        "MessagePane" ->
            Chat(
                app,
                channel,
                users,
                fixture.rows("channelUsers"),
                true,
                Modifier.fillMaxSize(),
                membershipOverride = if ("isMember" in args) args.flag("isMember") else null,
            )
        else -> error("Unregistered Kotlin message component: $component")
    }
}

@Composable
private fun StoryMessageList(
    app: Session,
    fixture: Row,
    args: Row,
    messages: List<Row>,
    users: List<Row>,
    channel: Row,
) {
    val counts = args.nested("replyCounts")
    val all =
        messages +
            messages.flatMap { row ->
                List(counts.text(row.text("id")).toIntOrNull() ?: 0) { index ->
                    obj("id" to "count-${row.text("id")}-${index}", "parent_id" to row.text("id"))
                }
            }
    CompositionLocalProvider(
        LocalAttachments provides
            (if ("attachments" in args) args.rows("attachments") else fixture.rows("attachments"))
    ) {
        MessageList(
            app,
            channel,
            all,
            fixture.rows("reactions"),
            users,
            if (fixture.text("component") == "ThreadPane") args.nested("root").text("id") else null,
            true,
            Modifier.fillMaxSize(),
            isLoading = args.flag("isLoading"),
            isError = args.flag("isError"),
            canCompose = args["canCompose"] == null || args.flag("canCompose"),
            editingId = args.text("editingId").ifEmpty { null },
            editingText = args.text("editingText"),
            canReact = args["canReact"] == null || args.flag("canReact"),
            retry =
                if (fixture.array("callbacks").any { it.jsonPrimitive.content == "onRetry" }) {
                    {
                        app.run {
                            app.platform.invoke(
                                "storyCallback",
                                storyPayload("onRetry", emptyList()),
                            )
                        }
                    }
                } else null,
            callbacks = { message -> storyMessageCallbacks(app, fixture, message) },
            readState = args.present("channelId"),
        )
    }
}

@Composable
private fun StoryMessage(
    app: Session,
    fixture: Row,
    args: Row,
    message: Row,
    users: List<Row>,
    channel: Row,
) {
    val item = args.nested("item")
    val actors =
        users +
            listOf(
                obj(
                    "id" to message.text("user_id"),
                    "display_name" to message.text("display_name"),
                    "user_name" to message.text("user_name"),
                )
            )
    val own =
        if (args.flag("isOwn"))
            JsonObject(message + ("user_id" to JsonPrimitive(app.config.userId)))
        else message
    val attachments =
        if ("attachments" in args) args.rows("attachments") else fixture.rows("attachments")
    val count =
        args.text("replyCount").toIntOrNull()
            ?: args.nested("replyCounts").text(message.text("id")).toIntOrNull()
            ?: 0
    val all =
        fixture.rows("messages") +
            List(count) { obj("id" to "reply-count-$it", "parent_id" to message.text("id")) }
    CompositionLocalProvider(LocalAttachments provides attachments) {
        Message(
            app,
            channel,
            own,
            all,
            fixture.rows("reactions"),
            actors,
            grouped = item["isGroupStart"] != null && !item.flag("isGroupStart"),
            wide = true,
            initialEditing = args.flag("isEditing") || args.text("editingId") == message.text("id"),
            initialEditingText =
                args.text("editingText").takeIf {
                    args.flag("isEditing") || args.text("editingId") == message.text("id")
                },
            groupEnd = item["isGroupEnd"] == null || item.flag("isGroupEnd"),
            canReact = args["canReact"] == null || args.flag("canReact"),
            callbacks = storyMessageCallbacks(app, fixture, message),
            canOpenThread = fixture.text("component") != "ThreadPane",
            ownOverride =
                if (fixture.text("component") == "MessageRow") args.flag("isOwn") else null,
        )
    }
}

private fun storyMessageCallbacks(app: Session, fixture: Row, message: Row): MessageCallbacks {
    fun callback(name: String, values: List<JsonElement> = emptyList()): (() -> Unit)? =
        if (fixture.array("callbacks").any { it.jsonPrimitive.content == name }) {
            { app.run { app.platform.invoke("storyCallback", storyPayload(name, values)) } }
        } else null
    val rowComponent = fixture.text("component") == "MessageRow"
    val messageId = JsonPrimitive(message.text("id"))
    val messageContent = JsonPrimitive(message.text("content"))
    return MessageCallbacks(
        startEdit =
            callback(
                "onStartEdit",
                if (rowComponent) emptyList() else listOf(messageId, messageContent),
            ),
        saveEdit = callback("onSaveEdit"),
        cancelEdit = callback("onCancelEdit"),
        requestDelete =
            callback("onRequestDelete", if (rowComponent) emptyList() else listOf(messageId)),
        openThread = callback("onOpenThread", if (rowComponent) emptyList() else listOf(messageId)),
        pickReaction =
            callback("onPickReaction", if (rowComponent) emptyList() else listOf(messageId)),
        reaction =
            if (fixture.array("callbacks").any { it.jsonPrimitive.content == "onReaction" })
                { emoji ->
                    callback(
                            "onReaction",
                            (if (rowComponent) emptyList() else listOf(messageId)) +
                                JsonPrimitive(emoji),
                        )
                        ?.invoke()
                }
            else null,
        changeText =
            if (
                fixture.array("callbacks").any { it.jsonPrimitive.content == "onEditingTextChange" }
            )
                { text -> callback("onEditingTextChange", listOf(JsonPrimitive(text)))?.invoke() }
            else null,
    )
}

private fun storyPayload(name: String, args: List<JsonElement>): String =
    buildJsonObject {
            put("name", name)
            put("args", JsonArray(args))
        }
        .toString()

@Composable
private fun StoryComposer(
    app: Session,
    fixture: Row,
    args: Row,
    users: List<Row>,
    channel: String,
    parent: String?,
    callback: String,
) {
    Composer(
        app,
        channel,
        parent,
        if ("mentionUsers" in args) args.rows("mentionUsers") else users,
        !args.flag("disabled") && (args["isMember"] == null || args.flag("isMember")),
        showTyping = fixture.text("component") != "Composer",
        placeholder = if ("placeholder" in args) args.text("placeholder") else null,
        ariaLabel = if ("ariaLabel" in args) args.text("ariaLabel") else null,
        commandOptions = if ("commands" in args) args.rows("commands") else null,
        mentionMemberIds =
            if ("mentionMemberIds" in args)
                args.array("mentionMemberIds").map { it.jsonPrimitive.content }.toSet()
            else null,
        onTyping =
            if (fixture.array("callbacks").any { it.jsonPrimitive.content == "onTyping" })
                { active ->
                    app.run {
                        app.platform.invoke(
                            "storyCallback",
                            storyPayload("onTyping", listOf(JsonPrimitive(active))),
                        )
                    }
                }
            else null,
        onRequestCommands =
            if (fixture.array("callbacks").any { it.jsonPrimitive.content == "onRequestCommands" })
                { ->
                    app.run {
                        app.platform.invoke(
                            "storyCallback",
                            storyPayload("onRequestCommands", emptyList()),
                        )
                    }
                }
            else null,
        allowAttachments =
            fixture.array("callbacks").any { it.jsonPrimitive.content == "onUploadAttachment" },
        sendMessage = { content, appId, attachments ->
            val values =
                when {
                    attachments.isNotEmpty() ->
                        listOf(
                            JsonPrimitive(content),
                            buildJsonObject { put("__zooUndefined", true) },
                            JsonArray(attachments.map(::JsonPrimitive)),
                        )
                    appId != null -> listOf(JsonPrimitive(content), JsonPrimitive(appId))
                    else -> listOf(JsonPrimitive(content))
                }
            app.platform.invoke("storyCallback", storyPayload(callback, values)) != "false"
        },
        uploadAttachment =
            if (fixture.array("callbacks").any { it.jsonPrimitive.content == "onUploadAttachment" })
                { file ->
                    val descriptor = buildJsonObject {
                        put("__zooFile", true)
                        put("name", file.text("name"))
                        put("type", file.text("type"))
                        put("base64", app.platform.invoke("fileRead", file.text("id")))
                    }
                    json
                        .parseToJsonElement(
                            app.platform.invoke(
                                "storyCallback",
                                storyPayload("onUploadAttachment", listOf(descriptor)),
                            )
                        )
                        .jsonObject
                }
            else null,
    )
}
