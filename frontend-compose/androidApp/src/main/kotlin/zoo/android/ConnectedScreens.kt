package zoo.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.firstOrNull
import kotlinx.serialization.json.JsonPrimitive
import zoo.*
import zoo.android.ui.*
import zoo.phone.PhoneController

@Composable
internal fun ConnectedScreens(
    session: Session,
    platform: AndroidPlatform,
    phone: PhoneController,
    enableNotifications: () -> Unit,
    background: () -> Unit,
) {
    val state = remember(session) { ConnectedScreenState(session, platform, phone) }
    DisposableEffect(state) { onDispose { state.close() } }
    val env = platform.env
    val channels by session.repo.channels.rows.collectAsState()
    val users by session.repo.users.rows.collectAsState()
    val memberships by session.repo.members.rows.collectAsState()
    val channelId = session.channelId
    val channel = channels.find { it.text("id") == channelId }
    val channelMembers = memberships.filter { it.text("channel_id") == channelId }
    val member = channelMembers.any { it.text("user_id") == session.config.userId }
    val rows = if (channelId == null) emptyList() else {
        val table = remember(channelId) { session.repo.messages(channelId) }
        DisposableEffect(channelId) { onDispose { session.repo.releaseChannel(channelId) } }
        val messages by table.rows.collectAsState()
        messages
    }
    val usersById = remember(users) { users.associateBy { it.text("id") } }
    val posts = state.home.posts.map { state.entry(it, usersById) }
    val encrypted = channel?.let { session.isEncryptedChannel(it.text("id")) } == true
    val messages by produceState(emptyList<TimelineItem>(), rows, usersById, encrypted) {
        value = rows.filterNot { it.present("deleted_at") }.map { row ->
            val data = if (row.present("ciphertext")) {
                val body = try { platform.messageEncryption?.decrypt(row)?.body }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { null }
                row + ("content" to JsonPrimitive(body ?: session.t("e2ee.sendUnavailable")))
            } else row
            state.entry(kotlinx.serialization.json.JsonObject(data), usersById)
        }
    }
    val currentThread = session.thread
    val conversationEntries = rows.filter { it.text("parent_id").isEmpty() }.map { it.text("id") }.toSet()
    val threadRoot = messages.find { it.id == currentThread }
    val threadReplies = rows.filter { it.text("parent_id") == currentThread }.map { it.text("id") }.toSet()
    val canPost = env.online && channel != null && member && !channel.present("archive_at")
    val route = env.path.substringBefore('?')
    val focusMessage = env.path.substringAfter("?message=", "").substringBefore('&').ifBlank { null }
    val focusParent = rows.find { it.text("id") == focusMessage }?.text("parent_id")
    LaunchedEffect(focusMessage, focusParent) {
        if (!focusParent.isNullOrBlank()) session.thread = focusParent
    }
    val call = phone.call
    val bubbleDraft by state.bubble.input.collectAsState()
    val bubbles by state.bubble.entries.collectAsState()
    LaunchedEffect(route, env.online, env.visible) {
        if (route == "/bubble" && env.online && env.visible) state.bubble.run()
    }
    LaunchedEffect(route, env.visible, env.online) {
        if (route == "/home" && env.online && env.visible) {
            state.home.restoreSelection()
            if (state.home.needsRefresh()) state.home.refresh(retain = true)
        }
        if (route == "/notifications" && env.online && env.visible) session.activityPubNotifications.ensureLoaded()
    }
    DisposableEffect(state, route, env.visible) {
        val unsubscribe = if (route == "/home" && env.visible) session.homeLive.subscribe { state.home.refresh(retain = true) } else null
        onDispose { unsubscribe?.invoke() }
    }
    fun back() {
        when {
            call != null -> background()
            currentThread != null -> session.thread = null
            state.overlay in setOf("members", "rename", "archive", "unarchive") -> state.overlay = "channel-settings"
            state.overlay == "add-member" -> state.overlay = "members"
            state.overlay != null -> state.overlay = null
            platform.back() -> Unit
            else -> background()
        }
    }
    BackHandler { back() }
    val screen = when {
        call != null -> AndroidScreen.Call(call.label, call.status == "incoming", call.status == "active", call.muted, call.speaker)
        state.overlay == "browse" -> AndroidScreen.Directory("チャンネルを探す", state.query,
            visibleChannels(channels).filterNot { it.flag("is_dm") }
                .filter { it.text("channel_name").contains(state.query, ignoreCase = true) }
                .map { SelectionItem(it.text("id"), it.text("channel_name"), it.text("description")) })
        state.overlay == "dm" -> AndroidScreen.Directory("ダイレクトメッセージを作成", state.query,
            users.filter { it.text("id") != session.config.userId && author(it).contains(state.query, ignoreCase = true) }
                .map { SelectionItem(it.text("id"), author(it), isUserName = true) })
        state.overlay == "add-member" -> AndroidScreen.Directory("メンバーを追加", state.query,
            users.filter { u -> channelMembers.none { it.text("user_id") == u.text("id") } && author(u).contains(state.query, true) }
                .map { SelectionItem(it.text("id"), author(it), isUserName = true) })
        state.overlay == "members" -> AndroidScreen.Directory("メンバー", state.query,
            users.filter { u -> channelMembers.any { it.text("user_id") == u.text("id") } && author(u).contains(state.query, true) }
                .map { SelectionItem(it.text("id"), author(it), isUserName = true) })
        state.overlay == "create" || state.overlay == "rename" -> AndroidScreen.EditChannel(
            if (state.overlay == "create") "チャンネルを作成" else "チャンネルを編集",
            state.channelName, state.channelDescription, if (state.overlay == "create") "作成" else "保存",
            env.online && state.channelName.isNotBlank() && !state.saving)
        state.overlay in setOf("channel-settings", "archive", "unarchive") && channel != null -> AndroidScreen.ChannelSettings(
            channel.text("channel_name"), state.channelSettings(channel, member, encrypted))
        state.overlay == "new-post" -> AndroidScreen.NewPost(
            InputState(state.postDraft, canSend = env.online && !state.saving && state.postFiles.canSend(state.postDraft)),
            state.recent, state.photosOpen, state.postVisibility,
            state.postFiles.items.map { RecentPhoto(it.file.text("id"), it.name) },
        )
        state.overlay == "post" && state.postDetail.isNotEmpty() -> {
            val entries = state.postDetail.map { state.entry(it, usersById) }
            AndroidScreen.Post(entries.first(), entries.drop(1), InputState(state.replyDraft, canSend = env.online && !state.saving))
        }
        state.overlay in setOf("theme", "language", "notifications", "search-engine") -> state.settingsDetail(env)
        route == "/home" -> AndroidScreen.Home(posts, state.home.timeline)
        route == "/notifications" -> state.inbox.screen()
        route == "/bubble" -> AndroidScreen.Bubble(bubbles.map { state.entry(it, usersById) }, InputState(bubbleDraft))
        route == "/settings" -> AndroidScreen.Settings(state.settings(env, phone.dmWaiting))
        channel != null && threadRoot != null -> AndroidScreen.Thread(
            channel.text("channel_name"), threadRoot, messages.filter { it.id in threadReplies },
            state.input("$channelId:$currentThread", canPost), focusMessage,
        )
        channel != null -> AndroidScreen.Conversation(channelId.orEmpty(), channel.text("channel_name"), channel.text("description"),
            messages.filter { it.id in conversationEntries }, state.input(channelId.orEmpty(), canPost),
            canCall = channel.flag("is_dm") && member && !encrypted, focusMessage = focusMessage)
        else -> AndroidScreen.Channels(state.channelSections(channels, usersById, memberships))
    }
    AndroidScreens(screen, env.dark, env.visible && env.focused,
        dispatch = { action ->
            when {
                action == AndroidUiAction.Back -> back()
                action is AndroidUiAction.SelectResult && state.overlay == "members" && channel != null &&
                    state.canEditMembers(channel) && action.id != session.config.userId -> {
                    usersById[action.id]?.let { state.memberToRemove = SelectionItem(action.id, author(it)) }
                }
                else -> state.dispatch(action, screen, channel, enableNotifications)
            }
        },
        language = env.locale, interaction = state.interaction.snapshot()) { item, modifier ->
        val image by produceState<ImageBitmap?>(null, item.id, state.photoBytes[item.id], state.postFiles.items.size) {
            value = state.postFiles.items.find { it.file.text("id") == item.id }?.bitmap
                ?: state.photoBytes[item.id]?.let { platform.imageFrames(it).firstOrNull() }
        }
        image?.let { Image(it, item.label, modifier, contentScale = ContentScale.Crop) }
            ?: Box(modifier.background(androidx.compose.material3.MaterialTheme.colorScheme.surfaceVariant),
                contentAlignment = Alignment.Center) { Text(item.label, maxLines = 1) }
    }
    state.inputMenu?.let { tool ->
        val emoji = tool == InputTool.Emoji
        val options = if (emoji) listOf("👍" to "👍", "😊" to "😊", "🎉" to "🎉", "❤️" to "❤️")
            else listOf("B" to "**太字**", "I" to "*斜体*", "‹/›" to "`コード`", "❝" to "\n> 引用")
        AlertDialog(
            onDismissRequest = { state.inputMenu = null },
            title = { Text(if (emoji) "絵文字" else "文字装飾") },
            text = {
                Row {
                    options.forEach { (label, value) ->
                        TextButton(onClick = { state.insertText(screen, channelId.orEmpty(), value) }) { Text(label) }
                    }
                }
            },
            confirmButton = {},
        )
    }
    state.memberToRemove?.let { user ->
        AlertDialog(
            onDismissRequest = { state.memberToRemove = null },
            title = { Text("チャンネルから削除") },
            text = { Text(user.title) },
            confirmButton = { TextButton(onClick = { channel?.let { state.removeMember(it, user.id) } }) { Text("削除") } },
            dismissButton = { TextButton(onClick = { state.memberToRemove = null }) { Text("キャンセル") } },
        )
    }
    val archive = state.overlay?.takeIf { it == "archive" || it == "unarchive" }
    if (archive != null && channel != null) {
        val label = if (archive == "archive") "アーカイブ" else "アーカイブを解除"
        AlertDialog(
            onDismissRequest = { state.overlay = "channel-settings" },
            title = { Text(label) },
            text = { Text(channel.text("channel_name")) },
            confirmButton = { TextButton(onClick = { state.archiveChannel(channel, archive) }) { Text(label) } },
            dismissButton = { TextButton(onClick = { state.overlay = "channel-settings" }) { Text("キャンセル") } },
        )
    }

}
