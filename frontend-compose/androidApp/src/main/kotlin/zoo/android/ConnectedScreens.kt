package zoo.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import io.ktor.http.Url
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.firstOrNull
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonPrimitive
import zoo.*
import zoo.android.ui.*
import zoo.phone.PhoneController
import zoo.android.phone.rememberCallImage

/** 窓の外の対象まで古い方へ広げる、1 ページずつの読み込みの上限。 */
private const val MAX_FOCUS_PAGES = 20
private const val MAX_FOCUS_FAILURES = 3

@Composable
internal fun ConnectedScreens(
    session: Session,
    platform: AndroidPlatform,
    phone: PhoneController,
    enableNotifications: () -> Unit,
    background: () -> Unit,
    callPresentation: Int = 0,
    openIncomingSettings: (() -> Unit)? = null,
    openPictureInPicture: ((String) -> Unit)? = null,
) {
    val state = remember(session) { ConnectedScreenState(session, platform, phone) }
    val pageState = key(session.config.apiBaseUrl, session.config.userId) { rememberSaveableStateHolder() }
    DisposableEffect(state) { onDispose { state.close() } }
    val openChannelLink = remember(state) { { target: ChannelLinkTarget, parent: String? -> state.openChannelLink(target, parent) } }
    val channelLinks = rememberChannelLinks(session, platform, openChannelLink)
    val env = platform.env
    val channels by session.repo.channels.rows.collectAsState()
    val users by session.repo.users.rows.collectAsState()
    val memberships by session.repo.members.rows.collectAsState()
    val channelId = session.channelId
    val channel = channels.find { it.text("id") == channelId }
    val channelMembers = memberships.filter { it.text("channel_id") == channelId }
    val member = channelMembers.any { it.text("user_id") == session.config.userId }
    val messageTable = if (channelId == null) null else remember(channelId) { session.repo.messages(channelId) }
    if (channelId != null) DisposableEffect(channelId) { onDispose { session.repo.releaseChannel(channelId) } }
    val rows = if (messageTable == null) emptyList() else {
        val messages by messageTable.rows.collectAsState()
        messages
    }
    val pageStates = if (messageTable == null) emptyMap() else {
        val states by messageTable.pageStates.collectAsState()
        states
    }
    // 添付・リアクションは、メッセージ行の display_data から出す（別 Shape は購読しない）。
    val reactions = remember(rows) { rows.flatMap(::displayReactions) }
    val context = androidx.compose.ui.platform.LocalContext.current.applicationContext
    val emojiCatalog = remember(context) { AndroidEmojiCatalog(context) }
    val needsWorkspaceEmoji = reactions.any { it.text("emoji").startsWith(':') }
    LaunchedEffect(session, needsWorkspaceEmoji, env.online, env.visible) {
        if (needsWorkspaceEmoji && env.online && env.visible) {
            try { session.loadWorkspaceEmojis() }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { /* Keep shortcode fallbacks if the catalog is unavailable. */ }
        }
    }
    val previewService by session.previews.summalyEndpoint.collectAsState()
    val usersById = remember(users) { users.associateBy { it.text("id") } }
    val posts = state.home.posts.map { state.entry(it, usersById, previewService = previewService) }
    val encrypted = channel?.let { session.isEncryptedChannel(it.text("id")) } == true
    val canPost = env.online && channel != null && member && !channel.present("archive_at")
    // 暗号ストアが変わったら（解錠、鍵の受信、鍵バックアップの取り込み）、表示中の行を復号し直す。
    val encryption = platform.messageEncryption
    val cryptoChanges by (encryption?.changes ?: remember { kotlinx.coroutines.flow.MutableStateFlow(0L) }).collectAsState()
    val messageEntries by produceState(emptyList<TimelineItem>(), rows, usersById, encrypted, canPost, cryptoChanges, session.workspaceEmojis) {
        state.forgetSearchPreviews(rows)
        val ordered = rows.filterNot { it.present("deleted_at") }.sortedWith(compareBy({ it.text("created_at") }, { it.text("id") }))
        value = ordered.map { row ->
            val decrypted = if (row.present("ciphertext")) {
                try { encryption?.decrypt(row) ?: DecryptedBody(notice = "e2ee.sendUnavailable") }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { DecryptedBody(notice = "e2ee.undecryptable.failed") }
            } else null
            val data = if (decrypted != null) row + ("content" to JsonPrimitive(decrypted.body.orEmpty())) else row
            val entry = state.entry(kotlinx.serialization.json.JsonObject(data), usersById,
                if (decrypted != null) emptyList() else displayAttachments(row),
                displayReactions(row), writable = canPost, allowPreview = !encrypted)
            encryptedEntry(entry, row, decrypted, encrypted, session::t)
        }
    }
    val messages = messageEntries.map { state.withLinkCard(it, previewService) }
    val rowsById = remember(rows) { rows.associateBy { it.text("id") } }
    val recipientNotices by (encryption?.recipients ?: remember { kotlinx.coroutines.flow.MutableStateFlow(emptyMap()) }).collectAsState()
    val recipientLines = if (!encrypted || channelId == null) emptyList() else recipientNoticeLines(recipientNotices[channelId],
        { id -> usersById[id]?.let(::author)?.ifBlank { null } ?: id }) { key, args -> session.t(key, *args) }
    val currentThread = session.thread
    val threadDetail = state.threadDetails.takeIf { it.text("id") == currentThread } ?: obj()
    val conversationEntries = rows.filter { it.text("parent_id").isEmpty() }.map { it.text("id") }.toSet()
    val threadRoot = messages.find { it.id == currentThread }
    val threadReplies = rows.filter { it.text("parent_id") == currentThread }.map { it.text("id") }.toSet()
    val route = env.path.substringBefore('?')
    var focusFailed by remember(env.path) { mutableStateOf(false) }
    val focusMessage = if (focusFailed) null
        else env.path.substringAfter("?message=", "").substringBefore('&').ifBlank { null }
    val focusRequest = focusMessage?.let { env.path.substringAfter('?') }
    val focusRow = rows.find { it.text("id") == focusMessage }
    val focusParent = focusRow?.text("parent_id")
    // A reply opened from search or a link stays readable when its parent was deleted (as Web): the thread
    // shows a notice in place of the parent and cannot be written to.
    val deletedRoot = currentThread != null && focusParent == currentThread && focusRow?.present("deleted_at") == false &&
        rows.find { it.text("id") == currentThread }?.present("deleted_at") == true
    LaunchedEffect(focusMessage, focusParent) {
        if (!focusParent.isNullOrBlank()) session.thread = focusParent
    }
    // 窓の外の対象（通知・リンク・検索）は、1 行だけ先に取ってから、含むまで古い方へ 1 ページずつ広げる（回数に上限あり）。
    LaunchedEffect(messageTable, focusRequest) {
        val table = messageTable ?: return@LaunchedEffect
        val target = focusMessage ?: return@LaunchedEffect
        if (table.rows.value.none { it.text("id") == target }) {
            // 通信の失敗は、間隔を空けて数回やり直す。取れなければ待たずに終え、利用者が開き直せるようにする。
            var found = false
            var tries = 0
            while (!found && tries < MAX_FOCUS_FAILURES) {
                found = table.loadAround(target)
                if (!found && ++tries < MAX_FOCUS_FAILURES) delay(500L * tries)
            }
            if (!found) {
                focusFailed = true
                session.toast = session.t("common.errors.network")
                return@LaunchedEffect
            }
        }
        // 行は、最初のメインのページが画面に反映されるまで rows に入らない。入るのを待ってから広げる。
        // 時間では打ち切らない（遅い回線でも、入った時点で広げる）。画面を離れる・対象が変わると、Effect ごと取り消される。
        val row = table.rows.first { rows -> rows.any { it.text("id") == target } }
            .first { it.text("id") == target }
        val range = row.text("parent_id").takeIf { it.isNotEmpty() }?.let(PageScope::Replies) ?: PageScope.Main
        // 上限は、取れたページ数と失敗の回数で数える。待機は数えない。
        var pages = 0
        var failures = 0
        while (pages < MAX_FOCUS_PAGES && failures < MAX_FOCUS_FAILURES) {
            val state = table.pageStates.value[range]
            if (pageCovers(state?.oldest, row.text("created_at"), row.text("id"))) return@LaunchedEffect
            if (state != null && state.loaded && !state.hasOlder) return@LaunchedEffect
            // ほかの処理が取っている間は、時間では打ち切らず待つ（取り消されるまで）。
            if (state?.loading == true || state == null) { delay(100); continue }
            if (table.loadOlder(range)) pages++ else { failures++; delay(100) }
        }
    }
    // スレッドの親（窓の外かもしれない）と、返信の最新の 1 ページ。
    LaunchedEffect(messageTable, currentThread) {
        val table = messageTable ?: return@LaunchedEffect
        val thread = currentThread ?: return@LaunchedEffect
        if (table.rows.value.none { it.text("id") == thread }) {
            // 親が取れないと、スレッド画面を選べない。対象行と同じく、数回やり直し、取れなければ通知して返信へは進まない。
            var found = false
            var tries = 0
            while (!found && tries < MAX_FOCUS_FAILURES) {
                found = table.loadAround(thread)
                if (!found && ++tries < MAX_FOCUS_FAILURES) delay(500L * tries)
            }
            if (!found) {
                session.toast = session.t("common.errors.network")
                return@LaunchedEffect
            }
        }
        // 親行は、最初のメインのページが画面に反映されるまで rows に入らない。入るのを待ってから返信を取る。
        table.rows.first { rows -> rows.any { it.text("id") == thread } }
        table.loadOlder(PageScope.Replies(thread))
    }
    val call = phone.call
    val callChannelId = call?.target ?: channelId
    val dmPeer = memberships.filter { it.text("channel_id") == callChannelId && it.text("user_id") != session.config.userId }
        .map { it.text("user_id") }.distinct().singleOrNull()
    val callUserId = if (call?.kind == "dm") call.peerUserId ?: dmPeer else null
    val caller = usersById[callUserId]
    val callAvatar = rememberCallImage(session, caller?.text("avatar_url"))
    var minimized by remember(call?.id, callPresentation) { mutableStateOf(false) }
    val callScreen = call?.let {
        AndroidScreen.Call(
            caller?.let(::author).orEmpty().ifBlank { call.label },
            call.status == "incoming", call.status == "active", call.muted, call.speaker,
            avatar = callAvatar, hasUser = callUserId != null, minimize = !phone.error,
            pictureInPicture = call.status == "active" && openPictureInPicture != null,
            status = session.t("sip.status.${call.status}"),
        )
    }
    val showFullCall = callScreen != null && (!minimized || callScreen.incoming || phone.error)
    val bubbleDraft by state.bubble.input.collectAsState()
    val bubbles by state.bubble.entries.collectAsState()
    val bubbleParticipants = remember(bubbles, usersById, session.config.userId) {
        val self = usersById[session.config.userId]?.let(::author).orEmpty().ifBlank { "自分" }
        listOf(SelectionItem(session.config.userId, self, "自分", isUserName = true)) +
            bubbles.filter { it.text("user_id") != session.config.userId }.map {
                SelectionItem(it.text("user_id"), author(it), isUserName = true)
            }
    }
    LaunchedEffect(state.overlay, state.openedPostId, state.postRevision,
        state.feedScreenRevision, state.postReloadRequest, env.online, env.visible) {
        if (env.online && env.visible) state.reloadActivePost()
    }
    LaunchedEffect(state.overlay, state.profile.text("id"), state.postRevision,
        state.feedScreenRevision, state.profileReloadRequest, env.online, env.visible) {
        if (env.online && env.visible) state.reloadActiveProfile()
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
    val search = session.globalSearch
    // Read the words from the same `env.path` as `route`: `session.env` follows a frame later, and its
    // previous route would clear the search when returning to it.
    val searchRoute = globalSearchRoute(env.path)
    val searching = searchRoute != null
    LaunchedEffect(searchRoute) {
        searchRoute?.let(search::followRoute)?.let { state.searchText = it }
    }
    LaunchedEffect(searching) {
        if (!searching) return@LaunchedEffect
        // Back from an opened result, the route still names the shown words, so `followRoute` changes nothing.
        // Drop words typed but not yet searched when leaving; otherwise they would replace the kept list and
        // its position. In-screen `replace` keeps `searching`, so the words being edited stay.
        searchRoute?.let { state.searchText = it.query }
        search.refreshIfStale()
    }
    LaunchedEffect(searching, state.searchText) {
        // Wait for typing to pause: each request may scan 12 months.
        if (!searching || state.searchText.trim() == search.query) return@LaunchedEffect
        delay(350)
        search.search(state.searchText)
    }
    LaunchedEffect(searching, search.query, search.type, search.filters) {
        // Keep the route restorable (back from an opened result) without adding history steps.
        val path = globalSearchPath(search.query, search.type, search.filters)
        if (searching && env.path != path) platform.replace(path)
    }
    val sipRoute = route == "/settings" && env.path.substringAfter('?', "").split('&').contains("tab=sip")
    fun back() {
        when {
            state.mediaPreview != null -> state.mediaPreview = null
            showFullCall -> background()
            sipRoute && state.overlay == null -> {
                state.invalidatePostNavigation()
                platform.navigate("/settings")
            }
            state.overlay == "edit-entry" -> state.finishEditing()
            state.overlay == "post" -> state.closePost()
            state.overlay == "profile" -> state.closeProfile()
            state.overlay == "thread-members" -> state.overlay = "thread-settings"
            state.overlay == "thread-settings" -> state.overlay = null
            state.overlay in setOf("members", "rename", "archive", "unarchive") -> state.overlay = "channel-settings"
            state.overlay == "add-member" -> state.overlay = "members"
            state.overlay != null -> state.overlay = null
            state.collapseInput(channelId, currentThread) -> Unit
            currentThread != null -> session.thread = null
            platform.back() -> state.invalidatePostNavigation()
            else -> {
                state.invalidatePostNavigation()
                background()
            }
        }
    }
    BackHandler { back() }
    // 他のユーザーの検証（届いた要求と、自分から始めた検証）は、どの画面にいても前面に出す。設定の「暗号化」から
    // 始めた検証も出せるよう、画面を切り替える早期の return より前に置く。
    VerificationHost(session, platform)
    if (call == null && state.overlay == "app-update") {
        pageState.SaveableStateProvider("app-update") {
            zoo.android.update.UpdateSettings(androidx.compose.ui.platform.LocalContext.current, env.dark, env.locale, ::back)
        }
        return
    }
    if (call == null && state.overlay == "encryption" && platform.e2ee != null) {
        pageState.SaveableStateProvider("encryption") { EncryptionRoute(session, platform, ::back) }
        return
    }
    val profile = state.profile
    val remoteProfile = profile.flag("remote")
    val profileUrl = profile.text("url").takeIf { publicUrl(it) != null }
    val profileHost = profileUrl?.let { Url(it).host }.orEmpty()
    val profileAvatarUrl = timelineAvatarUrl(session.config.apiBaseUrl, profile)
    val profileAvatar = if (state.overlay == "profile" && profileAvatarUrl != null) {
        key(session, profileAvatarUrl) {
            val initial = session.cachedImageBitmap(profileAvatarUrl)
            produceState(initial) {
                if (initial == null) {
                    value = try { session.imageBitmap(profileAvatarUrl) }
                    catch (cancelled: CancellationException) { throw cancelled }
                    catch (_: Exception) { null }
                }
            }.value
        }
    } else null
    val screen = when {
        showFullCall -> checkNotNull(callScreen)
        state.overlay == "people" -> AndroidScreen.Directory("ユーザーを検索", state.query,
            state.people.map { SelectionItem(it.text("id"), author(it), it.text("user_name"), isUserName = true) })
        state.overlay == "profile" -> AndroidScreen.Profile(
            author(profile), profile.text("summary"),
            state.profilePosts.map { state.entry(it, usersById, previewService = previewService) }, state.followState,
            handle = profile.text("user_name").takeIf { it.isNotBlank() }?.let { name ->
                if (remoteProfile && profileHost.isNotBlank()) "@$name@$profileHost" else "@$name"
            },
            avatar = profileAvatar, originalAvailable = profileUrl != null, id = profile.text("id"),
            postsPending = state.profilePostsPending, postsLoadFailed = state.profilePostsLoadFailed, online = env.online,
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
                .map { SelectionItem(it.text("id"), author(it), isUserName = true) }, scopeId = currentThread)
        state.overlay == "add-member" -> AndroidScreen.Directory("メンバーを追加", state.query,
            users.filter { u -> channelMembers.none { it.text("user_id") == u.text("id") } && author(u).contains(state.query, true) }
                .map { SelectionItem(it.text("id"), author(it), isUserName = true) }, scopeId = channelId)
        state.overlay == "members" -> AndroidScreen.Directory("メンバー", state.query,
            users.filter { u -> channelMembers.any { it.text("user_id") == u.text("id") } && author(u).contains(state.query, true) }
                .map { SelectionItem(it.text("id"), author(it), isUserName = true) }, scopeId = channelId)
        state.overlay == "create" || state.overlay == "rename" -> AndroidScreen.EditChannel(
            if (state.overlay == "create") "チャンネルを作成" else "チャンネルを編集",
            state.channelName, state.channelDescription, if (state.overlay == "create") "作成" else "保存",
            env.online && state.channelName.isNotBlank() && !state.saving,
            privateChannel = state.channelPrivate.takeUnless { state.overlay == "rename" && encrypted },
            sensitive = state.channelSensitive,
            encrypted = state.channelEncrypted.takeIf { state.overlay == "create" && state.channelPrivate },
        )
        state.overlay in setOf("channel-settings", "archive", "unarchive") && channel != null -> AndroidScreen.ChannelSettings(
            channel.text("channel_name"), state.channelSettings(channel, member, encrypted), channel.text("id"))
        state.overlay == "new-post" -> AndroidScreen.NewPost(
            InputState(state.postDraft, canSend = env.online && !state.saving && state.postFiles.canSend(state.postDraft), selection = state.postSelection),
            state.recent, state.photosOpen, state.postVisibility,
            state.postFiles.items.map { RecentPhoto(it.file.text("id"), it.name) },
            state.federatePost.takeIf { state.federationAvailable },
        )
        state.overlay == "edit-entry" -> AndroidScreen.EditEntry(
            InputState(state.editDraft, expanded = true, canSend = env.online && !state.saving && state.editDraft.isNotBlank(),
                selection = state.editSelection))
        state.overlay == "post" && state.postDetail.isNotEmpty() -> {
            val entries = state.postDetail.map { state.entry(it, usersById, previewService = previewService) }
            val root = entries.firstOrNull { it.id == state.openedPostId } ?: entries.first()
            AndroidScreen.Post(root, entries.filter { it.id != root.id }, InputState(state.replyDraft, canSend = env.online && !state.saving,
                selection = state.replySelection, focusRequest = state.replyFocusRequest, lastSend = state.replyLastSend),
                loadFailed = state.postLoadFailed, online = env.online)
        }
        state.overlay == "post" -> AndroidScreen.PostLoading(env.online, state.postLoadFailed)
        state.overlay in setOf("theme", "language", "notifications", "search-engine") -> state.settingsDetail(env)
        searching -> state.searchScreen(usersById, channels, memberships, previewService)
        route == "/home" -> AndroidScreen.Home(posts, state.home.timeline, state.home.canLoadOlder,
            loadFailed = state.home.error || state.home.olderError, loading = state.home.loading || state.home.updating, online = env.online)
        route == "/notifications" -> state.inbox.screen()
        route == "/bubble" -> AndroidScreen.Bubble(bubbles.map { state.entry(it, usersById) }, InputState(bubbleDraft, live = true), bubbleParticipants.map { it.title })
        route == "/settings" -> AndroidScreen.Settings(state.settings(env, openIncomingSettings != null))
        channel != null && (threadRoot != null || deletedRoot) -> {
          // 暗号化の境目は、スレッドの画面に出す親と返信の中で決める。
          val thread = withEncryptionBoundary(
              listOf(threadRoot ?: TimelineItem(currentThread.orEmpty(), "", session.t("threads.rootDeleted"), "")) +
                  // Under a deleted parent the thread is read-only (as Web): no edits, deletes or reactions.
                  messages.filter { it.id in threadReplies }.map {
                      if (threadRoot == null) it.copy(editable = false, canReact = false, canRepost = false) else it
                  },
              rowsById, encrypted, session::t)
          AndroidScreen.Thread(
            channel.text("channel_name"),
            thread.first(),
            thread.drop(1),
            state.input("$channelId:$currentThread", canPost && threadRoot != null && !threadDetail.present("thread_archive_at") &&
                (!threadDetail.flag("tracked") || threadDetail.text("membership") == "joined"),
                users.filter { user -> channelMembers.any { it.text("user_id") == user.text("id") } }).copy(notices = recipientLines), focusMessage,
            focusRequest = focusRequest,
            loadOlder = pageStates[PageScope.Replies(currentThread.orEmpty())]?.let { it.hasOlder && !it.loading } == true,
            historyReady = !env.online || pageStates[PageScope.Replies(currentThread.orEmpty())]?.loaded == true,
          )
        }
        channel != null -> AndroidScreen.Conversation(channelId.orEmpty(), channel.text("channel_name"), channel.text("description"),
            withEncryptionBoundary(messages.filter { it.id in conversationEntries }, rowsById, encrypted, session::t), state.input(channelId.orEmpty(), canPost,
                users.filter { user -> channelMembers.any { it.text("user_id") == user.text("id") } }).copy(notices = recipientLines),
            canCall = channel.flag("is_dm") && member && !encrypted, focusMessage = focusMessage, focusRequest = focusRequest,
            loadOlder = pageStates[PageScope.Main]?.let { it.hasOlder && !it.loading } == true,
            historyReady = !env.online || pageStates[PageScope.Main]?.loaded == true)
        else -> AndroidScreen.Channels(state.channelSections(channels, usersById, memberships))
    }
    val instanceHosts = when (screen) {
        is AndroidScreen.Home -> screen.posts
        is AndroidScreen.Profile -> screen.posts
        is AndroidScreen.Post -> listOf(screen.root) + screen.replies
        else -> emptyList()
    }.mapNotNull { entry ->
        entry.postHeader?.takeIf { it.remoteServer && it.channel == null }?.server
    }.distinct().sorted()
    LaunchedEffect(instanceHosts, env.online, env.visible) {
        if (env.online && env.visible) state.loadInstanceColors(instanceHosts)
    }
    val previewLinks = when (screen) {
        is AndroidScreen.Home -> screen.posts
        is AndroidScreen.Conversation -> screen.entries.takeLast(20)
        is AndroidScreen.Thread -> listOf(screen.root) + screen.replies.takeLast(20)
        is AndroidScreen.Post -> listOf(screen.root) + screen.replies
        is AndroidScreen.Profile -> screen.posts
        else -> emptyList()
    }.mapNotNull { it.link }.distinct().take(20)
    LaunchedEffect(previewService, previewLinks, env.online, env.visible) {
        if (previewService.isNotBlank() && env.online && env.visible) {
            for (url in previewLinks) session.previews.load(previewService, url)
        }
    }
    val compactCall = callScreen?.takeIf { !showFullCall }
    Column(Modifier.fillMaxSize().then(if (compactCall != null) Modifier.statusBarsPadding() else Modifier)) {
        if (compactCall != null) {
            val colors = AndroidColors(env.dark)
            Row(Modifier.fillMaxWidth().background(colors.background).padding(horizontal = 16.dp),
                horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
                Text(compactCall.status.orEmpty(), Modifier.weight(1f), color = colors.secondary)
                UiIconButton(UiIcon.Phone, session.t("androidPhone.expandCall"), colors.foreground,
                    { minimized = false }, fill = colors.quietSurface)
            }
        }
        Box(Modifier.weight(1f)) {
            if (!showFullCall && (state.overlay == "sip" || (state.overlay == null && sipRoute))) {
                pageState.SaveableStateProvider("sip") { SipRoute(session, phone, users, ::back) }
            } else {
                CompositionLocalProvider(
                    LocalTimelineImage provides remember(session) { { media -> timelineImage(session, media) } },
                    LocalTimelineImageRetry provides remember(session) { { media -> timelineImage(session, media, retry = true) } },
                    LocalCachedTimelineImage provides remember(session) { { media -> cachedTimelineImage(session, media) } },
                    LocalKitchenReactionImage provides emojiCatalog::kitchenReaction,
                    LocalAndroidTranslation provides { key, args -> session.t(key, *args) },
                    LocalChannelLinks provides channelLinks,
                ) {
                    AndroidScreens(screen, env.dark, env.visible && env.focused,
                        dispatch = { action ->
                            when {
                                action == AndroidUiAction.Back -> back()
                                action == AndroidUiAction.MinimizeCall -> minimized = true
                                action == AndroidUiAction.FloatCall -> call?.let { openPictureInPicture?.invoke(it.id) }
                                action == AndroidUiAction.OpenSetting("incoming-screen") -> openIncomingSettings?.invoke()
                                action is AndroidUiAction.SelectResult && state.overlay == "members" && channel != null &&
                                    state.canEditMembers(channel) && action.id != session.config.userId -> {
                                    usersById[action.id]?.let { state.memberToRemove = SelectionItem(action.id, author(it)) }
                                }
                                else -> state.dispatch(action, screen, channel, rows, dmPeer, usersById[dmPeer]?.let(::author), enableNotifications)
                            }
                        },
                        language = env.locale, interaction = state.interaction.snapshot(), pageState = pageState,
                        remoteAudioLevel = { phone.remoteAudioLevel },
                        loadNotificationReactions = state.inbox::loadReactions,
                        loadNotificationAvatar = { url ->
                            try { timelineImageUrl(session.config.apiBaseUrl, url)?.let { session.imageBitmap(it) } }
                            catch (cancelled: CancellationException) { throw cancelled }
                            catch (_: Exception) { null }
                        }) { item, modifier ->
                        val bytes = state.photoBytes[item.id]
                        val recentImage by key(item.id, bytes) {
                            produceState<ImageBitmap?>(null) {
                                value = bytes?.let { platform.imageFrames(it).firstOrNull() }
                            }
                        }
                        val image = state.postFiles.items.find { it.file.text("id") == item.id }?.bitmap ?: recentImage
                        image?.let { Image(it, item.label, modifier, contentScale = ContentScale.Crop) }
                            ?: Box(modifier.background(androidx.compose.material3.MaterialTheme.colorScheme.surfaceVariant),
                                contentAlignment = Alignment.Center) { Text(item.label, maxLines = 1) }
                    }
                }
            }
        }
    }
    val mediaSelection = state.mediaPreview
    // Observe the current Shapes so a change to channel visibility closes an open search image.
    val canKeepMedia = remember(mediaSelection, channels, memberships, users) {
        mediaSelection?.let(state::canKeepMediaPreview) ?: true
    }
    LaunchedEffect(env.visible, showFullCall, canKeepMedia) {
        if (!env.visible || showFullCall || !canKeepMedia) state.mediaPreview = null
    }
    mediaSelection?.takeIf { env.visible && !showFullCall && canKeepMedia }?.let { selection ->
        val target = selection.searchTarget
        if (target?.messageId != null) {
            LaunchedEffect(selection) {
                session.repo.linkedMessages.observe(target.channelId, checkNotNull(target.messageId)).collect {
                    state.dismissUnavailableSearchPreview(selection, it)
                }
            }
        }
        key(selection) {
            val media = selection.selected
            val dismiss = { state.mediaPreview = null }
            val openOriginal: (TimelineMedia) -> Unit = { original ->
                if (state.canKeepMediaPreview(selection)) {
                    val id = original.attachmentId
                    if (id != null) session.attachmentLinks.download(
                        obj("id" to id, "file_name" to original.name),
                        force = selection.searchTarget != null,
                    ) else original.url?.let { platform.action("open:$it") }
                } else dismiss()
            }
            if (media.kind == TimelineMediaKind.Image) {
                CompositionLocalProvider(
                    LocalTimelineImage provides remember(session) { { image -> timelineImage(session, image) } },
                    LocalTimelineImageRetry provides remember(session) { { image -> timelineImage(session, image, retry = true) } },
                    LocalCachedTimelineImage provides remember(session) { { image -> cachedTimelineImage(session, image) } },
                    LocalAndroidTranslation provides { key, args -> session.t(key, *args) },
                ) {
                    TimelineImageViewer(selection, dismiss, openOriginal)
                }
            } else {
                TimelineVideoPreview(session, media, AndroidColors(env.dark), dismiss) {
                    openOriginal(media)
                }
            }
        }
    }
    if (phone.error) {
        AlertDialog(
            onDismissRequest = phone::clearError,
            title = { Text(session.t("androidPhone.callError")) },
            confirmButton = { TextButton(onClick = phone::clearError) { Text(session.t("common.close")) } },
        )
    }
    val reaction = state.confirmingEntry?.takeIf { it.operation == EntryOperation.React }
    val insertingEmoji = state.inputMenu == InputTool.Emoji
    val recentEmojis = remember(session) { mutableStateListOf<EmojiChoice>() }
    if (insertingEmoji || reaction != null) {
        val unicode by produceState(emptyList<EmojiChoice>(), emojiCatalog) { value = emojiCatalog.unicode() }
        val allowAccountEmoji = reaction == null || screen !is AndroidScreen.Home &&
            screen !is AndroidScreen.Post && screen !is AndroidScreen.Profile
        LaunchedEffect(session, env.online, allowAccountEmoji) {
            if (env.online) {
                if (allowAccountEmoji) launch { session.loadAccountProfiles() }
                try {
                    session.loadWorkspaceEmojis()
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (_: Exception) {
                    // The bundled Unicode list remains available when the workspace catalog fails.
                }
            }
        }
        val dismiss = { state.inputMenu = null; state.confirmingEntry = null }
        EmojiPicker(
            title = if (reaction == null) "絵文字" else "リアクション",
            colors = AndroidColors(env.dark),
            unicode = unicode,
            custom = remember(session.workspaceEmojis, session.accountProfiles, allowAccountEmoji) {
                workspaceEmojiChoices(session.workspaceEmojis) +
                    (if (allowAccountEmoji) accountEmojiChoices(session.accountProfiles) else emptyList())
            },
            recent = recentEmojis.toList(),
            customLoading = session.workspaceEmojiLoading,
            customError = session.workspaceEmojiError,
            loadKitchen = emojiCatalog::kitchen,
            loadImage = { url ->
                try { timelineImageUrl(session.config.apiBaseUrl, url)?.let { session.imageBitmap(it) } }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { null }
            },
            dismiss = dismiss,
        ) { choice ->
            recentEmojis.removeAll { it.value == choice.value }
            recentEmojis.add(0, choice)
            if (recentEmojis.size > 24) recentEmojis.removeAt(recentEmojis.lastIndex)
            if (reaction != null) {
                dismiss()
                state.dispatch(reaction.copy(value = choice.value), screen, channel, rows, dmPeer, null, enableNotifications)
            } else {
                val value = if (choice.value.startsWith("kitchen:")) ":kitchen_${choice.value.removePrefix("kitchen:")}:" else choice.value
                state.insertText(screen, channelId.orEmpty(), value)
            }
        }
    }
    if (state.inputMenu == InputTool.Format) {
        FormattingDialog(onDismiss = { state.inputMenu = null }) { value ->
            state.insertText(screen, channelId.orEmpty(), value)
        }
    }
    state.confirmingEntry?.takeIf { it.operation == EntryOperation.Delete }?.let { action ->
        AlertDialog(
            onDismissRequest = { state.confirmingEntry = null },
            title = { Text("削除しますか？") },
            text = { Text(action.item.body, maxLines = 3) },
            confirmButton = {
                DialogAction("削除", UiIcon.Delete) {
                    state.confirmDelete(screen is AndroidScreen.Home || screen is AndroidScreen.Post || screen is AndroidScreen.Profile)
                }
            },
            dismissButton = { DialogAction("キャンセル", UiIcon.Close) { state.confirmingEntry = null } },
        )
    }
    state.memberToRemove?.let { user ->
        AlertDialog(
            onDismissRequest = { state.memberToRemove = null },
            title = { Text("チャンネルから削除") },
            text = { Text(user.title) },
            confirmButton = { DialogAction("削除", UiIcon.Delete) { channel?.let { state.removeMember(it, user.id) } } },
            dismissButton = { DialogAction("キャンセル", UiIcon.Close) { state.memberToRemove = null } },
        )
    }
    val archive = state.overlay?.takeIf { it == "archive" || it == "unarchive" }
    if (archive != null && channel != null) {
        val label = if (archive == "archive") "アーカイブ" else "アーカイブを解除"
        AlertDialog(
            onDismissRequest = { state.overlay = "channel-settings" },
            title = { Text(label) },
            text = { Text(channel.text("channel_name")) },
            confirmButton = { DialogAction(label, if (archive == "archive") UiIcon.Archive else UiIcon.Refresh) { state.archiveChannel(channel, archive) } },
            dismissButton = { DialogAction("キャンセル", UiIcon.Close) { state.overlay = "channel-settings" } },
        )
    }
}
