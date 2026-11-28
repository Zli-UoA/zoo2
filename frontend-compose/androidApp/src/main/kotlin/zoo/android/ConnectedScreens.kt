package zoo.android

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import androidx.activity.compose.BackHandler
import androidx.core.content.ContextCompat
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import io.ktor.http.Url
import java.time.LocalDate
import java.time.ZoneId
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.firstOrNull
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import zoo.*
import zoo.android.ui.*
import zoo.phone.PhoneController
import zoo.android.phone.rememberCallImage

/** 窓の外の対象まで古い方へ広げる、1 ページずつの読み込みの上限。 */
private const val MAX_FOCUS_PAGES = 20
private const val MAX_FOCUS_FAILURES = 3
private const val MAX_UNREAD_PAGES = 20

@Composable
internal fun ConnectedScreens(
    session: Session,
    platform: AndroidPlatform,
    phone: PhoneController,
    enableNotifications: () -> Unit,
    background: () -> Unit,
    settingsVersion: Int = 0,
    incoming: IncomingShare? = null,
    finishShare: () -> Unit = {},
    callPresentation: Int = 0,
    openIncomingSettings: (() -> Unit)? = null,
    openPictureInPicture: ((String) -> Unit)? = null,
) {
    val activityContext = androidx.compose.ui.platform.LocalContext.current
    val manager = remember(activityContext) { activityContext.getSystemService(android.app.NotificationManager::class.java) }
    val notificationOptions = remember(settingsVersion, manager) {
        listOf(SettingsEntry.Destination("os-notifications", "アプリの通知", if (manager.areNotificationsEnabled()) "許可" else "停止中")) +
            manager.notificationChannels.filterNot { it.id == "sample" }.map { channel ->
                SettingsEntry.Destination("os-channel/" + channel.id, channel.name.toString(),
                    if (channel.importance == android.app.NotificationManager.IMPORTANCE_NONE) "停止中" else "有効")
            }
    }
    val fullScreenAllowed = remember(settingsVersion, manager) {
        if (android.os.Build.VERSION.SDK_INT >= 34) manager.canUseFullScreenIntent() else null
    }
    val state = remember(session) { ConnectedScreenState(session, platform, phone) }
    val pageState = key(session.config.apiBaseUrl, session.config.userId) { rememberSaveableStateHolder() }
    DisposableEffect(state) { onDispose { state.close() } }
    val openChannelLink = remember(state) { { target: ChannelLinkTarget, parent: String? -> state.openChannelLink(target, parent) } }
    val channelLinks = rememberChannelLinks(session, platform, state::workspaceEmojiIndex, state::setLinkEmojiDemand, openChannelLink)
    val env = platform.env
    val channels by session.repo.channels.rows.collectAsState()
    val users by session.repo.users.rows.collectAsState()
    val memberships by session.repo.members.rows.collectAsState()
    val notificationSettings by session.repo.notificationSettings.rows.collectAsState()
    val membershipsLoading by session.repo.members.loading.collectAsState()
    val channelId = session.channelId
    val channelsLoading by session.repo.channels.loading.collectAsState()
    val syncedChannel = channels.find { it.text("id") == channelId }
    val pendingDm = state.pendingDm(channelId)
    val channel = syncedChannel ?: pendingDm
    val channelMembers = memberships.filter { it.text("channel_id") == channelId }
    val member = channelMembers.any { it.text("user_id") == session.config.userId }
    LaunchedEffect(channels, memberships, pendingDm) {
        state.clearSyncedDms(channels, memberships)
    }
    val dmSyncPending = pendingDm != null && (syncedChannel == null || !member)
    var dmSyncDelayed by remember(channelId) { mutableStateOf(false) }
    var dmSyncRetry by remember(channelId) { mutableIntStateOf(0) }
    LaunchedEffect(channelId, dmSyncPending, dmSyncRetry) {
        dmSyncDelayed = false
        if (dmSyncPending) {
            delay(15_000)
            dmSyncDelayed = true
        }
    }
    val messageTable = if (channelId == null) null else remember(channelId) { session.repo.messages(channelId) }
    val initialMessageSyncReady = messageTable?.initialSyncReady?.collectAsState()?.value == true
    val messageSyncFailed = messageTable?.error?.collectAsState()?.value != null
    val initialMessagePageFailed = messageTable?.initialPageFailed?.collectAsState()?.value == true
    LaunchedEffect(messageTable) {
        messageTable?.replicatedRows?.collect(state::reconcileOutgoing)
    }
    if (channelId != null) DisposableEffect(channelId) { onDispose { session.repo.releaseChannel(channelId) } }
    val rows = if (messageTable == null) emptyList() else {
        val messages by messageTable.rows.collectAsState()
        messages
    }
    val pageStates = if (messageTable == null) emptyMap() else {
        val states by messageTable.pageStates.collectAsState()
        states
    }
    val context = androidx.compose.ui.platform.LocalContext.current.applicationContext
    val uncategorizedEmoji = session.t("customEmoji.uncategorized")
    val emojiCatalog = remember(context, env.locale, uncategorizedEmoji) {
        AndroidEmojiCatalog(context, env.locale, uncategorizedEmoji)
    }
    val previewService by session.previews.summalyEndpoint.collectAsState()
    val usersById = remember(users) { users.associateBy { it.text("id") } }
    if (incoming != null) key(incoming, session) {
        val scope = rememberCoroutineScope()
        var busy by remember { mutableStateOf(false) }
        var error by remember { mutableStateOf<String?>(null) }
        val destinations = state.channelSections(channels, usersById, memberships)
        val resolver = androidx.compose.ui.platform.LocalContext.current.contentResolver
        ShareDestinationSheet(incoming.text, incoming.images.size, destinations, AndroidColors(env.dark), busy, error, finishShare, preview = {
            LazyRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                itemsIndexed(incoming.images, key = { _, uri -> uri.toString() }) { index, uri ->
                    val thumbnail by produceState<ImageBitmap?>(null, uri) {
                        value = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
                            try { resolver.shareThumbnail(uri)?.asImageBitmap() }
                            catch (e: CancellationException) { throw e }
                            catch (_: Exception) { null }
                        }
                    }
                    val label = "画像 ${index + 1}"
                    Box(Modifier.size(64.dp), contentAlignment = Alignment.Center) {
                        val bitmap = thumbnail
                        if (bitmap == null) Text(label, color = AndroidColors(env.dark).secondary)
                        else Image(bitmap, label, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
                    }
                }
            }
        }) { dest ->
            if (!busy) {
                busy = true
                scope.launch {
                    try {
                        error = state.receiveShare(incoming, dest)
                        if (error == null) finishShare()
                    } catch (e: CancellationException) { throw e }
                    catch (_: Exception) { error = "共有内容を開けませんでした" }
                    finally { busy = false }
                }
            }
        }
    }

    val mentionProfiles = remember(session.accountProfiles) {
        session.accountProfiles.associateBy { it.text("username") }
    }
    val mentionUsernames = remember(users) { users.map { it.text("user_name") }.toSet() }
    val posts = state.home.posts.map { state.entry(it, usersById, previewService = previewService) }
    val encrypted = channel?.let { session.isEncryptedChannel(it.text("id")) } == true
    val canPost = env.online && channel != null && member && !channel.present("archive_at")
    // 暗号ストアが変わったら（解錠、鍵の受信、鍵バックアップの取り込み）、表示中の行を復号し直す。
    val encryption = platform.messageEncryption
    val cryptoChanges by (encryption?.changes ?: remember { kotlinx.coroutines.flow.MutableStateFlow(0L) }).collectAsState()
    fun entry(row: Row, decrypted: DecryptedBody?): TimelineItem {
        val data = if (decrypted != null) JsonObject(row + ("content" to JsonPrimitive(decrypted.body.orEmpty()))) else row
        val item = state.entry(data, usersById,
            if (decrypted != null) emptyList() else displayAttachments(row),
            displayReactions(row), writable = canPost, allowPreview = !encrypted)
        val allowlist = decrypted?.mentions?.let { zoo.allowedMentionNames(it, usersById) }
        val entry = encryptedEntry(item, row, decrypted, encrypted, session::t)
        return entry.copy(
            allowedMentions = allowlist,
            mentionsCurrentUser = (decrypted != null || zoo.isPlaintextBeforeEncryption(row)) &&
                bodyMentionsUser(entry.body, usersById[session.config.userId]?.text("user_name").orEmpty(),
                    mentionUsernames, allowlist, entry.emoji.keys),
        )
    }
    val ordered = remember(rows) {
        rows.filterNot { it.present("deleted_at") }.sortedWith(compareBy({ it.text("created_at") }, { it.text("id") }))
    }
    val decryptedEntries by key(session, channelId) {
        produceState(emptyMap<Row, DecryptedBody>(), ordered, encryption, cryptoChanges) {
            value = ordered.filter { it.present("ciphertext") }.associateWith { row ->
                try { encryption?.decrypt(row) ?: DecryptedBody(notice = "e2ee.sendUnavailable") }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { DecryptedBody(notice = "e2ee.undecryptable.failed") }
            }
        }
    }
    LaunchedEffect(state, rows) { state.forgetSearchPreviews(rows) }
    val messageEntries = remember(ordered, usersById, encrypted, canPost, decryptedEntries, session.workspaceEmojis, env.locale) {
        ordered.map { row ->
            val decrypted = if (row.present("ciphertext"))
                decryptedEntries[row] ?: DecryptedBody(notice = "common.loading")
            else null
            entry(row, decrypted)
        }
    }
    val messages = messageEntries.map { state.withLinkCard(it, previewService) }
    val rowsById = remember(rows) { rows.associateBy { it.text("id") } }
    val recipientNotices by (encryption?.recipients ?: remember { kotlinx.coroutines.flow.MutableStateFlow(emptyMap()) }).collectAsState()
    val encryptionLines = if (!encrypted || channelId == null) emptyList() else recipientNoticeLines(recipientNotices[channelId],
        { id -> usersById[id]?.let(::author)?.ifBlank { null } ?: id }) { key, args -> session.t(key, *args) }
    val currentThread = session.thread
    var threadLoadFailed by remember(channelId, currentThread) { mutableStateOf(false) }
    var threadRetry by remember(channelId, currentThread) { mutableIntStateOf(0) }
    val threadDetail = state.threadDetails.takeIf { it.text("id") == currentThread } ?: obj()
    val recipientLines = encryptionLines + listOfNotNull(when {
        channel?.present("archive_at") == true -> "アーカイブ済み。履歴を閲覧できます。"
        currentThread != null && threadDetail.present("thread_archive_at") -> "このスレッドはアーカイブ済みです。"
        else -> null
    })
    val conversationEntries = rows.filter { it.text("parent_id").isEmpty() }.map { it.text("id") }.toSet()
    val currentReadPosition = session.channelReadPositions[channelId]
    var visitReadPosition by remember(session, channelId) { mutableStateOf<ChannelReadPosition?>(null) }
    LaunchedEffect(channelId, currentReadPosition) {
        if (visitReadPosition == null) visitReadPosition = currentReadPosition
    }
    val visitRead = visitReadPosition ?: currentReadPosition
    val readBoundary = visitRead?.timestamp
    var unreadPositioned by remember(session, channelId) { mutableStateOf(false) }
    val onUnreadPositioned = remember(session, channelId) { { unreadPositioned = true } }
    var visibleReadThrough by remember(session, channelId) { mutableStateOf<String?>(null) }
    val onReadThrough = remember(session, channelId) { { id: String? -> visibleReadThrough = id } }
    val threadRoot = messages.find { it.id == currentThread }
    val threadReplies = rows.filter { it.text("parent_id") == currentThread }.map { it.text("id") }.toSet()
    val route = env.path.substringBefore('?')
    var focusFailed by remember(env.path) { mutableStateOf(false) }
    val focusMessage = env.path.substringAfter("?message=", "").substringBefore('&').takeIf {
        it.isNotBlank() && !focusFailed && rowsById[it]?.present("deleted_at") != true
    }
    val focusRequest = focusMessage?.let { env.path.substringAfter('?') }
    val emptyGeneration = remember { MutableStateFlow(0) }
    val generation by (messageTable?.pageGeneration ?: emptyGeneration).collectAsState()
    val mainPage = pageStates[PageScope.Main]
    val cachedMainPage = if (messageTable == null) null else {
        val cached by messageTable.cachedMainPage.collectAsState()
        cached
    }
    val visiblePage = if (env.online || mainPage?.loaded == true) mainPage else cachedMainPage
    var unreadPages by remember(messageTable, generation) { mutableIntStateOf(0) }
    var unreadHistoryFailed by remember(messageTable, generation, env.online) { mutableStateOf(false) }
    val firstUnreadInWindow = remember(ordered, readBoundary, session.config.userId) {
        if (readBoundary == null) null else ordered.firstOrNull {
            it.text("parent_id").isEmpty() && it.text("user_id") != session.config.userId &&
                (timestampMillis(it.text("created_at")) ?: Long.MIN_VALUE) > readBoundary
        }?.text("id")
    }
    val latestOtherTimestamp = remember(ordered, session.config.userId) {
        ordered.asSequence()
            .filter { it.text("parent_id").isEmpty() && it.text("user_id") != session.config.userId }
            .mapNotNull { timestampMillis(it.text("created_at")) }
            .maxOrNull()
    }
    val visitHistoryPosition = visitRead?.let { it.copy(hasUnread = it.hasUnread || firstUnreadInWindow != null) }
    val positionForHistory = if (unreadPositioned) currentReadPosition?.let { saved ->
        saved.copy(hasUnread = saved.hasUnread || latestOtherTimestamp?.let { it > saved.timestamp } == true)
    } else visitHistoryPosition
    val unreadHistoryOutsideWindow = unreadHistoryOutsideWindow(visiblePage, positionForHistory)
    val needsUnreadHistory = unreadHistoryOutsideWindow && unreadPages < MAX_UNREAD_PAGES && !unreadHistoryFailed
    val unreadHistoryIncomplete = unreadHistoryOutsideWindow &&
        (!needsUnreadHistory || !env.online || focusMessage != null)
    val mainPageReady = mainPage?.loaded == true
    val awaitingInitialRead = env.online && channel?.present("archive_at") != true &&
        (member || membershipsLoading) && (if (currentReadPosition == null)
            !session.channelReadPositionsChecked else !mainPageReady)
    val unreadPositionPending = !unreadPositioned && (awaitingInitialRead ||
        (env.online && currentThread == null && focusMessage == null && needsUnreadHistory))
    val canLoadUnreadHistory = channel != null && currentThread == null && env.online && env.visible &&
        focusMessage == null && positionForHistory?.hasUnread == true
    val waitingForTarget = focusMessage != null && rows.none { it.text("id") == focusMessage }
    val loadingDetail = when {
        !env.online || (mainPageReady && !unreadPositionPending && !waitingForTarget) -> null
        messageSyncFailed -> session.t("threads.loadingDetails.syncError")
        !initialMessageSyncReady -> session.t("threads.loadingDetails.sync")
        initialMessagePageFailed -> session.t("threads.loadingDetails.pageRetry")
        !mainPageReady -> session.t("threads.loadingDetails.messages")
        waitingForTarget -> session.t("threads.loadingDetails.position")
        awaitingInitialRead -> session.t("threads.loadingDetails.readPosition")
        needsUnreadHistory && canLoadUnreadHistory -> session.t("threads.loadingDetails.unread")
        else -> session.t("threads.loadingDetails.position")
    }
    val unreadId = firstUnreadInWindow.takeUnless {
        unreadHistoryOutsideWindow(visiblePage, visitHistoryPosition) || awaitingInitialRead
    }
    val localDay by rememberConversationDay(context)
    val conversationWindow = remember(messages, conversationEntries, rowsById, encrypted, session) {
        withEncryptionBoundary(messages.filter { it.id in conversationEntries }, rowsById, encrypted, session::t)
    }
    val conversationWithOutgoing = if (channelId != null && currentThread == null)
        state.withOutgoing(channelId, conversationWindow) else conversationWindow
    val conversationMessages = remember(conversationWithOutgoing, unreadId, env.locale, localDay, session) {
        withConversationDividers(
            conversationWithOutgoing, unreadId, platform::day, session::dayLabel, session.t("messages.new"),
        )
    }
    val focusRow = focusMessage?.let(rowsById::get)
    val focusParent = focusRow?.text("parent_id")
    // Keep a deleted parent's thread readable and prevent replies to it.
    val deletedRoot = currentThread?.let { rowsById[it]?.present("deleted_at") } == true
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
    LaunchedEffect(messageTable, currentThread, env.online, env.visible, threadRetry) {
        if (!env.online || !env.visible) return@LaunchedEffect
        threadLoadFailed = false
        val table = messageTable ?: return@LaunchedEffect
        val thread = currentThread ?: return@LaunchedEffect
        if (table.rows.value.none { it.text("id") == thread }) {
            // 親を読み込みながらスレッド画面を表示し、対象行と同様に上限付きで再試行する。
            var found = false
            var tries = 0
            while (!found && tries < MAX_FOCUS_FAILURES) {
                found = table.loadAround(thread)
                if (!found && ++tries < MAX_FOCUS_FAILURES) delay(500L * tries)
            }
            if (!found) {
                threadLoadFailed = true
                return@LaunchedEffect
            }
        }
        // 親行は、最初のメインのページが画面に反映されるまで rows に入らない。入るのを待ってから返信を取る。
        table.rows.first { rows -> rows.any { it.text("id") == thread } }
        val replies = PageScope.Replies(thread)
        if (table.pageStates.value[replies]?.loaded != true) table.loadOlder(replies)
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
    LaunchedEffect(state.overlay, state.profile.text("id"), state.feedScreenRevision, state.profileAccountRetry, env.online, env.visible) {
        if (env.online && env.visible) state.reloadProfileAccount()
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
    }
    LaunchedEffect(route, env.online, env.visible) {
        if (route == "/bubble" && env.online && env.visible) state.bubble.run()
    }
    LaunchedEffect(route, env.visible, env.online) {
        if (route == "/home" && env.visible) {
            state.home.restoreSelection()
            if (env.online && state.home.needsRefresh()) state.home.refresh(retain = true)
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
            currentThread != null -> {
                // A notification for a reply has no target in the channel's timeline.
                if (focusParent == currentThread) platform.replace(route)
                session.thread = null
            }
            platform.back() -> Unit
            else -> {
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
            accountLoadFailed = state.profileAccountLoadFailed, followBusy = state.followBusy,
        )
        state.overlay == "bubble-participants" -> AndroidScreen.Directory("参加者", state.query,
            bubbleParticipants.filter { it.title.contains(state.query, true) })
        state.overlay == "browse" -> AndroidScreen.Directory("チャンネルを探す", state.query,
            visibleChannels(channels).filterNot { it.flag("is_dm") }
                .filter { it.text("channel_name").contains(state.query, ignoreCase = true) }
                .map { SelectionItem(it.text("id"), it.text("channel_name"), it.text("description")) })
        state.overlay == "dm" -> AndroidScreen.Directory("ダイレクトメッセージを作成", state.query,
            users.filter { it.text("id") != session.config.userId && author(it).contains(state.query, ignoreCase = true) }
                .map { SelectionItem(it.text("id"), author(it), it.text("user_name"), isUserName = true, selected = it.text("id") in state.dmTargets) },
            "開始", env.online && !state.saving && state.dmTargets.isNotEmpty(), state.encryptedDm)
        state.overlay == "thread-settings" && currentThread != null ->
            AndroidScreen.SettingsDetail("スレッド設定", state.threadSettings(currentThread, canPost, member))
        state.overlay == "thread-members" -> AndroidScreen.Directory("スレッドの参加者", state.query,
            state.threadDetails.array("participants").rows().filter { author(it).contains(state.query, true) }
                .map { SelectionItem(it.text("id"), author(it), it.text("user_name"), isUserName = true) }, scopeId = currentThread)
        state.overlay == "add-member" -> {
            val existing = channelMembers.mapTo(mutableSetOf()) { it.text("user_id") }
            val canInvite = channel != null && state.canEditMembers(channel)
            AndroidScreen.Directory("メンバーを追加", state.query, users.filter {
                author(it).contains(state.query, true) || it.text("user_name").contains(state.query, true)
            }.map { user ->
                val id = user.text("id")
                SelectionItem(id, author(user), user.text("user_name"), isUserName = true, unavailable = when {
                    id in existing -> "参加済み"
                    !canInvite -> "招待権限なし"
                    else -> null
                })
            }.sortedBy { it.unavailable != null }, scopeId = channelId)
        }
        state.overlay == "members" -> AndroidScreen.Directory("メンバー", state.query,
            users.filter { u -> channelMembers.any { it.text("user_id") == u.text("id") } && author(u).contains(state.query, true) }
                .map { SelectionItem(it.text("id"), author(it), it.text("user_name"), isUserName = true) }, scopeId = channelId)
        state.overlay == "create" || state.overlay == "rename" -> AndroidScreen.EditChannel(
            if (state.overlay == "create") "チャンネルを作成" else "チャンネルを編集",
            state.channelName, state.channelDescription, if (state.overlay == "create") "作成" else "保存",
            env.online && state.channelName.isNotBlank() && !state.saving,
            privateChannel = state.channelPrivate.takeUnless { state.overlay == "rename" && encrypted },
            saveError = state.channelSaveError,
            sensitive = state.channelSensitive,
            encrypted = state.channelEncrypted.takeIf { state.overlay == "create" && state.channelPrivate },
        )
        state.overlay in setOf("channel-settings", "archive", "unarchive") && channel != null -> AndroidScreen.ChannelSettings(
            channel.text("channel_name"),
            state.channelSettings(channel, member, encrypted,
                channelNotificationLevel(notificationSettings, channel.text("id"))),
            channel.text("id"))
        state.overlay == "new-post" -> AndroidScreen.NewPost(
            InputState(state.postDraft, canSend = env.online && !state.saving && state.postFiles.canSend(state.postDraft), selection = state.postSelection),
            state.recent, state.photosOpen, state.postVisibility,
            state.postFiles.items.map { RecentPhoto(it.file.text("id"), it.name, it.failed, it.description, it.sensitive) },
            state.federatePost.takeIf { state.federationAvailable }, quoting = state.quoteDraft,
        )
        state.overlay == "edit-entry" -> AndroidScreen.EditEntry(
            InputState(state.editDraft, expanded = true, canSend = env.online && !state.saving && state.editDraft.isNotBlank(),
                selection = state.editSelection, characterLimit = 40_000), original = state.editing?.item?.body)
        state.overlay == "post" && state.postDetail.isNotEmpty() -> {
            val entries = state.postDetail.map { state.entry(it, usersById, previewService = previewService) }
            val root = entries.firstOrNull { it.id == state.openedPostId } ?: entries.first()
            AndroidScreen.Post(root, entries.filter { it.id != root.id }, InputState(state.replyDraft, canSend = env.online && !state.saving && root.postHeader?.audience != null,
                characterLimit = 1000, selection = state.replySelection, focusRequest = state.replyFocusRequest, lastSend = state.replyLastSend),
                loadFailed = state.postLoadFailed, online = env.online, loading = state.postPending)
        }
        state.overlay == "post" -> AndroidScreen.PostLoading(env.online, state.postLoadFailed)
        state.overlay in setOf("theme", "language", "notifications", "search-engine", "navigation-cache", "attachment-cache") -> state.settingsDetail(env, notificationOptions)
        searching -> state.searchScreen(usersById, channels, memberships, previewService)
        route == "/home" -> AndroidScreen.Home(state.withOutgoing("/home", posts, newestFirst = true), state.home.timeline, state.home.canLoadOlder,
            showingSavedHead = state.home.showingSavedHead, loadFailed = state.home.error || state.home.olderError, loading = state.home.loading || state.home.updating, online = env.online)
        route == "/notifications" -> state.inbox.screen(state.followResponses)
        route == "/bubble" -> AndroidScreen.Bubble(bubbles.map { state.entry(it, usersById) }, InputState(bubbleDraft, live = true, characterLimit = 1000), bubbleParticipants.map { it.title })
        route == "/settings" -> AndroidScreen.Settings(state.settings(env, fullScreenAllowed))
        channel != null && (threadRoot != null || deletedRoot) -> {
          // 暗号化の境目は、スレッドの画面に出す親と返信の中で決める。
          val thread = withEncryptionBoundary(
              listOf(threadRoot ?: TimelineItem(currentThread.orEmpty(), "", session.t("threads.rootDeleted"), "")) +
                  // Under a deleted parent the thread is read-only (as Web): no edits, deletes or reactions.
                  messages.filter { it.id in threadReplies }.map {
                      if (threadRoot == null) it.copy(editable = false, canReact = false, canRepost = false) else it
                  },
              rowsById, encrypted, session::t)
          val threadFocus = focusMessage.takeIf { it == currentThread || focusParent == currentThread }
          AndroidScreen.Thread(
            channel.text("channel_name"),
            thread.first(),
            state.withOutgoing("$channelId:$currentThread", thread.drop(1)),
            state.input("$channelId:$currentThread", canPost && threadRoot != null && !threadDetail.present("thread_archive_at") &&
                (!threadDetail.flag("tracked") || threadDetail.text("membership") == "joined"),
                users.filter { user -> channelMembers.any { it.text("user_id") == user.text("id") } }).copy(notices = recipientLines), threadFocus,
            focusRequest = focusRequest.takeIf { threadFocus != null },
            loadOlder = pageStates[PageScope.Replies(currentThread.orEmpty())]?.let { it.hasOlder && !it.loading } == true,
            historyReady = !env.online || pageStates[PageScope.Replies(currentThread.orEmpty())]?.loaded == true,
          )
        }
        channel != null && currentThread != null -> AndroidScreen.OpeningConversation(
            "thread:$currentThread", session.t("threads.title"), env.online, threadLoadFailed) { threadRetry++ }
        channel != null -> AndroidScreen.Conversation(channelId.orEmpty(), channel.text("channel_name"), channel.text("description"),
            conversationMessages, state.input(channelId.orEmpty(), canPost,
                users.filter { user -> channelMembers.any { it.text("user_id") == user.text("id") } }).copy(notices = recipientLines),
            canCall = channel.flag("is_dm") && member && !encrypted, focusMessage = focusMessage, focusRequest = focusRequest,
            loadOlder = pageStates[PageScope.Main]?.let { it.hasOlder && !it.loading } == true,
            historyReady = !env.online || mainPageReady,
            loadingDetail = loadingDetail,
            unreadPositionPending = unreadPositionPending,
            unreadHistoryIncomplete = unreadHistoryIncomplete,
            onUnreadPositioned = onUnreadPositioned.takeIf { env.online && currentReadPosition != null && !unreadPositioned },
            onReadThrough = onReadThrough.takeIf { unreadPositioned },
            retrySync = if (dmSyncDelayed && env.online) {
                { session.repo.channels.refresh(); session.repo.members.refresh(); dmSyncRetry++ }
            } else null)
        channelId != null && channelsLoading -> AndroidScreen.OpeningConversation(
            "channel:$channelId", session.t("common.loading"), env.online,
            loadingDetail = session.t("threads.loadingDetails.channel"))
        else -> AndroidScreen.Channels(state.channelSections(channels, usersById, memberships))
    }
    val typingEnabled = env.visible && env.focused && canPost && (screen is AndroidScreen.Conversation ||
        (screen is AndroidScreen.Thread && threadRoot != null && !threadDetail.present("thread_archive_at") &&
            (!threadDetail.flag("tracked") || threadDetail.text("membership") == "joined")))
    val typing = remember(session, channelId, currentThread, typingEnabled) {
        if (typingEnabled && channelId != null) ChannelTyping(session, channelId, currentThread) else null
    }
    DisposableEffect(typing) { onDispose { typing?.close() } }
    LaunchedEffect(typing) { typing?.poll() }
    val typingLabel = typing?.label.orEmpty()
    val visibleScreen = when {
        typingLabel.isBlank() -> screen
        screen is AndroidScreen.Conversation -> screen.copy(input = screen.input.copy(notices = screen.input.notices + typingLabel))
        screen is AndroidScreen.Thread -> screen.copy(input = screen.input.copy(notices = screen.input.notices + typingLabel))
        else -> screen
    }
    val loadUnreadOnConversation = screen is AndroidScreen.Conversation && canLoadUnreadHistory
    LaunchedEffect(messageTable, generation, positionForHistory, loadUnreadOnConversation) {
        if (!loadUnreadOnConversation || messageTable == null) return@LaunchedEffect
        while (unreadPages < MAX_UNREAD_PAGES && !unreadHistoryFailed) {
            val page = messageTable.pageStates.first { states ->
                states[PageScope.Main]?.let { it.loaded && !it.loading } == true
            }.getValue(PageScope.Main)
            if (!unreadHistoryOutsideWindow(page, positionForHistory)) break
            if (messageTable.loadOlder(PageScope.Main)) unreadPages++ else unreadHistoryFailed = true
        }
    }
    val readEnabled = screen is AndroidScreen.Conversation && member && env.online &&
        env.visible && env.focused && currentReadPosition != null && unreadPositioned &&
        !unreadPositionPending && !unreadHistoryOutsideWindow && mainPageReady
    LaunchedEffect(session, channelId, messageTable, readEnabled) {
        if (!readEnabled || channelId == null || messageTable == null) return@LaunchedEffect
        snapshotFlow { visibleReadThrough }
            .combine(messageTable.replicatedRows) { id, confirmed ->
                // A locally inserted message can be used as a cursor only after sync confirms it.
                if (id == null) return@combine null
                val row = confirmed.firstOrNull { it.text("id") == id } ?: return@combine null
                if (row.present("deleted_at") || row.present("parent_id")) return@combine null
                timestampMillis(row.text("created_at"))?.let { id to it }
            }
            .distinctUntilChanged()
            .collect { cursor ->
                val (id, timestamp) = cursor ?: return@collect
                try {
                    session.markChannelRead(channelId, id, timestamp)
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    // Reaching the end again or returning to the screen retries the write.
                }
            }
    }
    val reaction = state.confirmingEntry?.takeIf { it.operation == EntryOperation.React }
    val insertingEmoji = state.inputMenu == InputTool.Emoji
    val pickerOpen = insertingEmoji || reaction != null
    val allowAccountEmoji = reaction == null || screen !is AndroidScreen.Home &&
        screen !is AndroidScreen.Post && screen !is AndroidScreen.Profile
    val usesAccountProfiles = when (screen) {
        is AndroidScreen.Conversation, is AndroidScreen.Thread, is AndroidScreen.Notifications,
        is AndroidScreen.Home, is AndroidScreen.Post, is AndroidScreen.Profile, is AndroidScreen.Bubble -> true
        else -> pickerOpen && allowAccountEmoji
    }
    LaunchedEffect(session, env.online, env.visible, usesAccountProfiles) {
        if (!env.online || !env.visible || !usesAccountProfiles) return@LaunchedEffect
        // Share the directory with the picker and stop refreshing when this screen is hidden.
        while (true) {
            session.loadAccountProfiles()
            delay(60_000)
        }
    }
    val needsWorkspaceEmoji = when (screen) {
        is AndroidScreen.Home -> screen.posts.any { it.needsWorkspaceEmoji }
        is AndroidScreen.Profile -> screen.posts.any { it.needsWorkspaceEmoji }
        is AndroidScreen.Post -> screen.root.needsWorkspaceEmoji || screen.replies.any { it.needsWorkspaceEmoji }
        is AndroidScreen.Thread -> screen.root.needsWorkspaceEmoji || screen.replies.any { it.needsWorkspaceEmoji }
        is AndroidScreen.Conversation -> screen.entries.any { it.needsWorkspaceEmoji }
        is AndroidScreen.Bubble -> screen.entries.any { it.needsWorkspaceEmoji }
        is AndroidScreen.Search -> screen.state.results.any { it.needsWorkspaceEmoji }
        else -> false
    }
    val canLoadEmoji = env.online && env.visible
    val pickerEmojiDemand by rememberUpdatedState(pickerOpen)
    val entryEmojiDemand by rememberUpdatedState(needsWorkspaceEmoji)
    LaunchedEffect(session, state, canLoadEmoji) {
        if (!canLoadEmoji) return@LaunchedEffect
        snapshotFlow { Triple(pickerEmojiDemand, entryEmojiDemand, state.linkEmojiDemandVersion) }.collect { (picker, entries, _) ->
            if (picker || entries || state.linkEmojiDemandCount > 0) {
                try { session.loadWorkspaceEmojis() }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { /* Keep shortcode fallbacks if the catalog is unavailable. */ }
            }
        }
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
    val linkedPosts = rememberPostPreviews(session, env.visible && env.online, platform.navigationCacheLimit)
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
                    LocalTimelineImageWithPreview provides remember(session) {
                        { media, onCached -> timelineImage(session, media, onCached = onCached) }
                    },
                    LocalTimelineImageRetry provides remember(session) { { media -> timelineImage(session, media, retry = true) } },
                    LocalAttachmentImageDenied provides remember(session) { session.attachmentLinks::isDenied },
                    LocalCachedTimelineImage provides remember(session) { { media -> cachedTimelineImage(session, media) } },
                    LocalKitchenReactionImage provides emojiCatalog::kitchenReaction,
                    LocalAndroidTranslation provides { key, args -> session.t(key, *args) },
                    LocalChannelLinks provides channelLinks,
                    LocalPostPreviews provides linkedPosts,
                    LocalMentionProfiles provides mentionProfiles,
                    LocalMentionUsernames provides mentionUsernames,
                ) {
                    AndroidScreens(visibleScreen, env.dark, env.visible && env.focused,
                        dispatch = { action ->
                            if (action is AndroidUiAction.TextChanged) typing?.edit(action.text.isNotBlank())
                            if (action == AndroidUiAction.UseInputTool(InputTool.Send)) typing?.edit(false)
                            when {
                                action == AndroidUiAction.Back -> back()
                                action == AndroidUiAction.MinimizeCall -> minimized = true
                                action == AndroidUiAction.FloatCall -> call?.let { openPictureInPicture?.invoke(it.id) }
                                action is AndroidUiAction.OpenSetting && action.id.startsWith("os-channel/") -> {
                                    val settings = Intent(android.provider.Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                                        .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName)
                                        .putExtra(android.provider.Settings.EXTRA_CHANNEL_ID, action.id.substringAfter('/'))
                                    try { activityContext.startActivity(settings) }
                                    catch (_: android.content.ActivityNotFoundException) { enableNotifications() }
                                }
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
                    LocalTimelineImageWithPreview provides remember(session) {
                        { media, onCached -> timelineImage(session, media, onCached = onCached) }
                    },
                    LocalTimelineImageRetry provides remember(session) { { image -> timelineImage(session, image, retry = true) } },
                    LocalAttachmentImageDenied provides remember(session) { session.attachmentLinks::isDenied },
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
            confirmButton = { DialogAction(session.t("common.close"), UiIcon.Close, click = phone::clearError) },
        )
    }
    val recentEmojis = remember(session) { mutableStateListOf<String>() }
    if (pickerOpen) {
        val unicode by produceState(emptyList<EmojiChoice>(), emojiCatalog) { value = emojiCatalog.unicode() }
        val dismiss = { state.inputMenu = null; state.confirmingEntry = null }
        EmojiPicker(
            title = if (reaction == null) "絵文字" else "リアクション",
            colors = AndroidColors(env.dark),
            unicode = unicode,
            custom = remember(session.workspaceEmojis, session.accountProfiles, allowAccountEmoji, env.locale) {
                workspaceEmojiChoices(session.workspaceEmojis, uncategorizedEmoji) +
                    (if (allowAccountEmoji) accountEmojiChoices(session.accountProfiles, session.t("reactions.accountEmojiCategory")) else emptyList())
            },
            recent = recentEmojis.toList(),
            recentCategory = session.t("reactions.recent"),
            customLoading = session.workspaceEmojiLoading,
            customError = session.workspaceEmojiError,
            retryCustom = { session.scope.launch { session.loadWorkspaceEmojis(force = true) } },
            loadKitchen = emojiCatalog::kitchen,
            prefetchCustomImages = { images ->
                val urls = images.mapNotNull { normalizedImageSource(session.config.apiBaseUrl, it)?.toString() }.distinct()
                prefetchEmojiImages(urls) { session.image(it) }
            },
            loadImage = { url ->
                try { normalizedImageSource(session.config.apiBaseUrl, url)?.toString()?.let { session.imageBitmap(it) } }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { null }
            },
            dismiss = dismiss,
        ) { choice ->
            if (choice.image == null && choice.glyph.isNotBlank()) {
                recentEmojis.remove(choice.value)
                recentEmojis.add(0, choice.value)
                if (recentEmojis.size > 24) recentEmojis.removeAt(recentEmojis.lastIndex)
            }
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
        val status = state.operationStatus(action)
        val sending = status == EntryOperationStatus.Sending
        AlertDialog(
            onDismissRequest = { if (!sending) state.confirmingEntry = null },
            title = { Text("削除しますか？") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(action.item.body, maxLines = 3)
                    when (status) {
                        EntryOperationStatus.Sending -> Text("削除中…")
                        EntryOperationStatus.Offline -> Text("接続してから再試行してください")
                        EntryOperationStatus.Denied -> Text("この投稿を削除する権限がありません")
                        EntryOperationStatus.Failed -> Text("削除できませんでした。再試行できます")
                        null -> Unit
                    }
                }
            },
            confirmButton = {
                DialogAction("削除", UiIcon.Delete, enabled = !sending) {
                    state.confirmDelete(screen is AndroidScreen.Home || screen is AndroidScreen.Post || screen is AndroidScreen.Profile)
                }
            },
            dismissButton = { DialogAction("キャンセル", UiIcon.Close, enabled = !sending) { state.confirmingEntry = null } },
        )
    }
    state.attachmentAction?.let { action ->
        val file = action.item.attachments.firstOrNull { it.first == action.value }
        val size = action.item.attachmentSizes[action.value]?.takeIf { it.toLongOrNull() != null }?.let(::attachmentSize)
        AttachmentExportDialog(file?.second ?: "添付ファイル", size, { state.attachmentAction = null }, state::exportAttachment)
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
    state.leaveTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { state.leaveTarget = null },
            title = { Text(if (target.second == null) "チャンネルから退出" else "スレッドから退出") },
            text = { Text(if (target.second == null) "「${channel?.text("channel_name").orEmpty()}」が参加中の一覧から外れます。"
                else "このスレッドの通知を終了します。チャンネルには引き続き参加します。") },
            confirmButton = { DialogAction("退出", UiIcon.Back) { state.leave(channelId, currentThread) } },
            dismissButton = { DialogAction("キャンセル", UiIcon.Close) { state.leaveTarget = null } },
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


private fun currentConversationDay(): Pair<ZoneId, LocalDate> {
    val zone = ZoneId.systemDefault()
    return zone to LocalDate.now(zone)
}

@Composable
private fun rememberConversationDay(context: Context): State<Pair<ZoneId, LocalDate>> =
    produceState(currentConversationDay(), context) {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                value = currentConversationDay()
            }
        }
        val filter = IntentFilter(Intent.ACTION_TIMEZONE_CHANGED).apply {
            addAction(Intent.ACTION_DATE_CHANGED)
            addAction(Intent.ACTION_TIME_CHANGED)
        }
        ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        value = currentConversationDay()
        awaitDispose { context.unregisterReceiver(receiver) }
    }
