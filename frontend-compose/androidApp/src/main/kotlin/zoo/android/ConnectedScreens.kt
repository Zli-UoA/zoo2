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
import io.ktor.http.Url
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
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
    val files = if (channelId == null) emptyList() else {
        val table = remember(channelId) { session.repo.attachments(channelId) }
        val values by table.rows.collectAsState()
        values
    }
    val reactions = if (channelId == null) emptyList() else {
        val table = remember(channelId) { session.repo.reactions(channelId) }
        val values by table.rows.collectAsState()
        values
    }
    val filesByMessage = remember(files) { files.groupBy { it.text("message_id") } }
    val reactionsByMessage = remember(reactions) { reactions.groupBy { it.text("message_id") } }
    val previewEndpoints by session.previews.endpoints.collectAsState()
    val previewService = previewEndpoints.first
    val usersById = remember(users) { users.associateBy { it.text("id") } }
    val posts = state.home.posts.map { state.entry(it, usersById, previewService = previewService) }
    val encrypted = channel?.let { session.isEncryptedChannel(it.text("id")) } == true
    val canPost = env.online && channel != null && member && !channel.present("archive_at")
    val messages by produceState(emptyList<TimelineItem>(), rows, usersById, encrypted, filesByMessage, reactionsByMessage, canPost) {
        value = rows.filterNot { it.present("deleted_at") }.sortedBy { it.text("created_at") }.map { row ->
            val data = if (row.present("ciphertext")) {
                val body = try { platform.messageEncryption?.decrypt(row)?.body }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { null }
                row + ("content" to JsonPrimitive(body ?: session.t("e2ee.sendUnavailable")))
            } else row
            val id = row.text("id")
            state.entry(kotlinx.serialization.json.JsonObject(data), usersById,
                filesByMessage[id].orEmpty(), reactionsByMessage[id].orEmpty(), previewService, writable = canPost, allowPreview = !encrypted)
        }
    }
    val currentThread = session.thread
    val threadDetail = state.threadDetails.takeIf { it.text("id") == currentThread } ?: obj()
    val conversationEntries = rows.filter { it.text("parent_id").isEmpty() }.map { it.text("id") }.toSet()
    val threadRoot = messages.find { it.id == currentThread }
    val threadReplies = rows.filter { it.text("parent_id") == currentThread }.map { it.text("id") }.toSet()
    val route = env.path.substringBefore('?')
    val focusMessage = env.path.substringAfter("?message=", "").substringBefore('&').ifBlank { null }
    val focusParent = rows.find { it.text("id") == focusMessage }?.text("parent_id")
    LaunchedEffect(focusMessage, focusParent) {
        if (!focusParent.isNullOrBlank()) session.thread = focusParent
    }
    val call = phone.call
    val callChannelId = call?.target ?: channelId
    val dmPeer = memberships.filter { it.text("channel_id") == callChannelId && it.text("user_id") != session.config.userId }
        .map { it.text("user_id") }.distinct().singleOrNull()
    val callUserId = if (call?.kind == "dm") call.peerUserId ?: dmPeer else null
    val caller = usersById[callUserId]
    val avatarUrl = caller?.text("avatar_url").orEmpty()
    val callAvatar = key(call?.id, callUserId, avatarUrl) {
        produceState<ImageBitmap?>(null) {
            if (avatarUrl.isNotBlank()) {
                value = try { session.imageBitmap(avatarUrl) }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { null }
            }
        }.value
    }
    val bubbleDraft by state.bubble.input.collectAsState()
    val bubbles by state.bubble.entries.collectAsState()
    val bubbleParticipants = remember(bubbles, usersById, session.config.userId) {
        val self = usersById[session.config.userId]?.let(::author).orEmpty().ifBlank { "自分" }
        listOf(SelectionItem(session.config.userId, self, "自分", isUserName = true)) +
            bubbles.filter { it.text("user_id") != session.config.userId }.map {
                SelectionItem(it.text("user_id"), author(it), isUserName = true)
            }
    }
    LaunchedEffect(route, env.online, env.visible) {
        if (env.online && env.visible) try { state.refreshPins() }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { }
    }
    LaunchedEffect(currentThread, env.online, env.visible) {
        if (currentThread != null && env.online && env.visible) try { state.loadThread(currentThread) }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { session.toast = session.t("common.errors.network") }
    }
    LaunchedEffect(state.query, state.overlay, env.online) {
        if (state.overlay == "people" && env.online) {
            delay(300)
            try { state.searchPeople(state.query) }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { session.toast = session.t("common.errors.network") }
        }
    }
    LaunchedEffect(state.overlay, env.online) {
        if (state.overlay == "search-engine" && env.online) session.searchPreferences.ensureLoaded()
        if (state.overlay == "new-post" && env.online) try { state.loadFederation() }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { }
    }
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
            state.overlay == "edit-entry" -> state.finishEditing()
            state.overlay == "post" -> state.closePost()
            state.overlay == "profile" -> state.overlay = "people"
            state.overlay == "thread-members" -> state.overlay = "thread-settings"
            state.overlay == "thread-settings" -> state.overlay = null
            state.overlay in setOf("members", "rename", "archive", "unarchive") -> state.overlay = "channel-settings"
            state.overlay == "add-member" -> state.overlay = "members"
            state.overlay != null -> state.overlay = null
            currentThread != null -> session.thread = null
            platform.back() -> Unit
            else -> background()
        }
    }
    BackHandler { back() }
    if (call == null && state.overlay == "app-update") {
        zoo.android.update.UpdateSettings(androidx.compose.ui.platform.LocalContext.current, env.dark, env.locale, ::back)
        return
    }
    if (call == null && state.overlay == "encryption" && platform.e2ee != null) {
        EncryptionRoute(session, platform, ::back)
        return
    }
    val profile = state.profile
    val remoteProfile = profile.flag("remote")
    val profileUrl = profile.text("url").takeIf { publicUrl(it) != null }
    val profileHost = profileUrl?.let { Url(it).host }.orEmpty()
    val profileAvatarUrl = if (remoteProfile) {
        profile.nested("avatar").text("image_id").takeIf { it.matches(Regex("[a-f0-9]{64}")) }
            ?.let { "/api/activitypub/emoji-images/$it" }
    } else profile.text("avatar_url").takeIf { it.startsWith("https://") && publicUrl(it) != null }
    val profileAvatar = if (state.overlay == "profile" && profileAvatarUrl != null) {
        key(profile.text("id"), profileAvatarUrl) {
            produceState<ImageBitmap?>(null) {
                value = try { session.imageBitmap(profileAvatarUrl) }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { null }
            }.value
        }
    } else null
    val screen = when {
        call != null -> AndroidScreen.Call(
            caller?.let(::author).orEmpty().ifBlank { call.label },
            call.status == "incoming", call.status == "active", call.muted, call.speaker,
            avatar = callAvatar, hasUser = callUserId != null,
        )
        state.overlay == "people" -> AndroidScreen.Directory("ユーザーを検索", state.query,
            state.people.map { SelectionItem(it.text("id"), author(it), it.text("user_name"), isUserName = true) })
        state.overlay == "profile" -> AndroidScreen.Profile(
            author(profile), profile.text("summary"),
            state.profilePosts.map { state.entry(it, usersById, previewService = previewService) }, state.followState,
            handle = profile.text("user_name").takeIf { it.isNotBlank() }?.let { name ->
                if (remoteProfile && profileHost.isNotBlank()) "@$name@$profileHost" else "@$name"
            },
            avatar = profileAvatar, originalAvailable = profileUrl != null,
        )
        state.overlay == "bubble-participants" -> AndroidScreen.Directory("参加者", state.query,
            bubbleParticipants.filter { it.title.contains(state.query, true) })
        state.overlay == "browse" -> AndroidScreen.Directory("チャンネルを探す", state.query,
            visibleChannels(channels).filterNot { it.flag("is_dm") }
                .filter { it.text("channel_name").contains(state.query, ignoreCase = true) }
                .map { SelectionItem(it.text("id"), it.text("channel_name"), it.text("description")) })
        state.overlay == "dm" -> AndroidScreen.Directory("ダイレクトメッセージを作成", state.query,
            users.filter { it.text("id") != session.config.userId && author(it).contains(state.query, ignoreCase = true) }
                .map { SelectionItem(it.text("id"), author(it), isUserName = true, selected = it.text("id") in state.dmTargets) },
            "開始", env.online && !state.saving && state.dmTargets.isNotEmpty(), state.encryptedDm)
        state.overlay == "thread-settings" && currentThread != null ->
            AndroidScreen.SettingsDetail("スレッド設定", state.threadSettings(currentThread, canPost, member))
        state.overlay == "thread-members" -> AndroidScreen.Directory("スレッドの参加者", state.query,
            state.threadDetails.array("participants").rows().filter { author(it).contains(state.query, true) }
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
            env.online && state.channelName.isNotBlank() && !state.saving,
            privateChannel = state.channelPrivate.takeUnless { state.overlay == "rename" && encrypted },
            sensitive = state.channelSensitive,
            encrypted = state.channelEncrypted.takeIf { state.overlay == "create" && state.channelPrivate },
        )
        state.overlay in setOf("channel-settings", "archive", "unarchive") && channel != null -> AndroidScreen.ChannelSettings(
            channel.text("channel_name"), state.channelSettings(channel, member, encrypted))
        state.overlay == "new-post" -> AndroidScreen.NewPost(
            InputState(state.postDraft, canSend = env.online && !state.saving && state.postFiles.canSend(state.postDraft)),
            state.recent, state.photosOpen, state.postVisibility,
            state.postFiles.items.map { RecentPhoto(it.file.text("id"), it.name) },
            state.federatePost.takeIf { state.federationAvailable && state.postVisibility != "local" },
        )
        state.overlay == "edit-entry" -> AndroidScreen.EditEntry(
            InputState(state.editDraft, expanded = true, canSend = env.online && !state.saving && state.editDraft.isNotBlank(),
                selection = state.editSelection))
        state.overlay == "post" && state.postDetail.isNotEmpty() -> {
            val entries = state.postDetail.map { state.entry(it, usersById, previewService = previewService) }
            val root = entries.firstOrNull { it.id == state.openedPostId } ?: entries.first()
            AndroidScreen.Post(root, entries.filter { it.id != root.id }, InputState(state.replyDraft, canSend = env.online && !state.saving, selection = state.replySelection))
        }
        state.overlay in setOf("theme", "language", "notifications", "search-engine") -> state.settingsDetail(env)
        route == "/home" -> AndroidScreen.Home(posts, state.home.timeline, state.home.canLoadOlder)
        route == "/notifications" -> state.inbox.screen()
        route == "/bubble" -> AndroidScreen.Bubble(bubbles.map { state.entry(it, usersById) }, InputState(bubbleDraft, live = true), bubbleParticipants.map { it.title })
        route == "/settings" -> AndroidScreen.Settings(state.settings(env, phone.dmWaiting))
        channel != null && threadRoot != null -> AndroidScreen.Thread(
            channel.text("channel_name"), threadRoot, messages.filter { it.id in threadReplies },
            state.input("$channelId:$currentThread", canPost && !threadDetail.present("thread_archive_at") &&
                (!threadDetail.flag("tracked") || threadDetail.text("membership") == "joined"),
                users.filter { user -> channelMembers.any { it.text("user_id") == user.text("id") } }), focusMessage,
        )
        channel != null -> AndroidScreen.Conversation(channelId.orEmpty(), channel.text("channel_name"), channel.text("description"),
            messages.filter { it.id in conversationEntries }, state.input(channelId.orEmpty(), canPost,
                users.filter { user -> channelMembers.any { it.text("user_id") == user.text("id") } }),
            canCall = channel.flag("is_dm") && member && !encrypted, focusMessage = focusMessage)
        else -> AndroidScreen.Channels(state.channelSections(channels, usersById, memberships))
    }
    val previewLinks = when (screen) {
        is AndroidScreen.Home -> screen.posts
        is AndroidScreen.Conversation -> screen.entries.takeLast(20)
        is AndroidScreen.Thread -> listOf(screen.root) + screen.replies.takeLast(20)
        is AndroidScreen.Post -> listOf(screen.root) + screen.replies
        else -> emptyList()
    }.mapNotNull { it.link }.distinct().take(20)
    LaunchedEffect(previewService, previewLinks, env.online, env.visible) {
        if (previewService.isNotBlank() && env.online && env.visible) {
            for (url in previewLinks) session.previews.load(previewService, url)
        }
    }
    AndroidScreens(screen, env.dark, env.visible && env.focused,
        dispatch = { action ->
            when {
                action == AndroidUiAction.Back -> back()
                action is AndroidUiAction.SelectResult && state.overlay == "members" && channel != null &&
                    state.canEditMembers(channel) && action.id != session.config.userId -> {
                    usersById[action.id]?.let { state.memberToRemove = SelectionItem(action.id, author(it)) }
                }
                else -> state.dispatch(action, screen, channel, rows, dmPeer, usersById[dmPeer]?.let(::author), enableNotifications)
            }
        },
        language = env.locale, interaction = state.interaction.snapshot(),
        remoteAudioLevel = { phone.remoteAudioLevel }) { item, modifier ->
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
    state.confirmingEntry?.let { action ->
        val deleting = action.operation == EntryOperation.Delete
        AlertDialog(
            onDismissRequest = { state.confirmingEntry = null },
            title = { Text(if (deleting) "削除しますか？" else "リアクション") },
            text = {
                if (deleting) Text(action.item.body, maxLines = 3)
                else Row {
                    listOf("👍", "❤️", "🎉", "😊").forEach { emoji ->
                        TextButton(onClick = {
                            state.dispatch(action.copy(value = emoji), screen, channel, rows, dmPeer, null, enableNotifications)
                        }) { Text(emoji) }
                    }
                }
            },
            confirmButton = {
                if (deleting) TextButton(onClick = {
                    state.confirmDelete(screen is AndroidScreen.Home || screen is AndroidScreen.Post || screen is AndroidScreen.Profile)
                }) { Text("削除") }
            },
            dismissButton = { TextButton(onClick = { state.confirmingEntry = null }) { Text("キャンセル") } },
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
