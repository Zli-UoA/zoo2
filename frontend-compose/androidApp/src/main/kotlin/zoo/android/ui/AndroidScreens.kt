package zoo.android.ui


import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.gestures.BringIntoViewSpec
import androidx.compose.foundation.gestures.LocalBringIntoViewSpec
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.SaveableStateHolder
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import androidx.compose.ui.unit.sp

internal enum class EntryOperation { Edit, Delete, React, Repost, Attachment, Link, Media, Quote }

internal data class TimelineItem(
    val id: String, val author: String, val body: String, val time: String,
    val editable: Boolean = false, val canReact: Boolean = false, val canRepost: Boolean = false,
    val reposted: Boolean = false, val reactions: List<TimelineReaction> = emptyList(),
    val attachments: List<Pair<String, String>> = emptyList(), val link: String? = null,
    val entryKey: String = id,
    val media: List<TimelineMedia> = emptyList(),
    val quote: TimelineQuote? = null, val repostedBy: String? = null,
    val postHeader: PostHeaderData? = null,
    val richContent: EntryRichContent = EntryRichContent(),
    val linkCard: EntryCard? = null,
    /** 本文に書いた、このアプリのチャンネル・メッセージへのリンク（埋め込みカードを出す。外部には送らない）。 */
    val channelLinks: List<String> = emptyList(),
    val emoji: Map<String, TimelineMedia> = emptyMap(),
    val needsWorkspaceEmoji: Boolean = false,
    val avatar: String? = null,
    val attachedImageUrls: Set<String> = emptySet(),
    /** 復号できなかった理由（docs/e2ee.md の「受信時の検証」）。本文の代わりに出す。 */
    val notice: String? = null,
    /** 送信者の確かさ・版・暗号化されていない行の印（本文と一緒に出す）。 */
    val marks: List<String> = emptyList(),
    /** この行の前に出す区切り（暗号化に切り替えた境目、#367）。 */
    val divider: String? = null,
    /** 暗号化した添付（`attachment_id` → 本文の `attachments` の値の JSON。鍵を含むので表示と取得にだけ使う）。 */
    val encryptedFiles: Map<String, String> = emptyMap(),
    val postAuthor: zoo.Row? = null,
    val dateDivider: String? = null,
    val unreadDivider: String? = null,
)
internal data class ChannelItem(
    val id: String,
    val title: String,
    val unread: Boolean = false,
    val isUserName: Boolean = false,
)
internal data class ChannelSection(val label: String, val channels: List<ChannelItem>)

internal sealed interface AndroidScreen {
    data class Home(
        val posts: List<TimelineItem>, val timeline: String? = null, val loadOlder: Boolean = false,
        val loadFailed: Boolean = false, val loading: Boolean = false, val online: Boolean = true,
    ) : AndroidScreen
    data class Channels(val sections: List<ChannelSection>) : AndroidScreen
    data class Notifications(val items: List<InboxItem>, val nextCursor: String? = null, val loadFailed: Boolean = false) : AndroidScreen
    data class Bubble(val entries: List<TimelineItem>, val input: InputState, val participants: List<String> = emptyList()) : AndroidScreen
    data class Settings(val groups: List<SettingsGroup>) : AndroidScreen
    data class Conversation(
        val id: String, val name: String, val description: String,
        val entries: List<TimelineItem>, val input: InputState, val canCall: Boolean = false,
        val focusMessage: String? = null,
        /** Changes on each request to show [focusMessage] (a search result opened again), unlike the id. */
        val focusRequest: String? = focusMessage,
        /** 古いページが残っていて、いま取れる。 */
        val loadOlder: Boolean = false,
        val historyReady: Boolean = true,
        val retrySync: (() -> Unit)? = null,
        val unreadPositionPending: Boolean = false,
        val onUnreadPositioned: (() -> Unit)? = null,
        val onReadThrough: ((String?) -> Unit)? = null,
    ) : AndroidScreen
    data class Thread(
        val channelName: String, val root: TimelineItem, val replies: List<TimelineItem>, val input: InputState,
        val focusMessage: String? = null,
        /** Changes on each request to show [focusMessage] (a search result opened again), unlike the id. */
        val focusRequest: String? = focusMessage,
        val loadOlder: Boolean = false,
        val historyReady: Boolean = true,
    ) : AndroidScreen
    data class OpeningConversation(
        val key: String, val title: String, val online: Boolean, val failed: Boolean = false,
        val retry: (() -> Unit)? = null,
    ) : AndroidScreen
    data class PostLoading(val online: Boolean, val failed: Boolean) : AndroidScreen
    data class Post(
        val root: TimelineItem, val replies: List<TimelineItem>, val input: InputState,
        val loadFailed: Boolean = false, val online: Boolean = true, val loading: Boolean = false,
    ) : AndroidScreen
    data class ChannelSettings(val channel: String, val groups: List<SettingsGroup>, val id: String = channel) : AndroidScreen
    data class SettingsDetail(val title: String, val groups: List<SettingsGroup>) : AndroidScreen
    data class EditChannel(
        val title: String,
        val name: String,
        val description: String,
        val submitLabel: String,
        val canSubmit: Boolean,
        val privateChannel: Boolean? = null,
        val sensitive: Boolean? = null,
        val encrypted: Boolean? = null,
    ) : AndroidScreen
    data class Directory(
        val title: String, val query: String, val results: List<SelectionItem>,
        val submit: String? = null, val canSubmit: Boolean = false, val encryption: Boolean? = null,
        val scopeId: String? = null,
    ) : AndroidScreen
    data class EditEntry(val input: InputState) : AndroidScreen
    data class Search(val state: SearchScreenState) : AndroidScreen
    data class Profile(
        val name: String, val bio: String, val posts: List<TimelineItem>, val follow: String? = null,
        val handle: String? = null, val avatar: ImageBitmap? = null,
        val originalAvailable: Boolean = false,
        val id: String = handle ?: name,
        val postsPending: Boolean = false, val postsLoadFailed: Boolean = false, val online: Boolean = true,
        val accountLoadFailed: Boolean = false,
    ) : AndroidScreen
    data class NewPost(val input: InputState, val recent: List<RecentPhoto>, val showPhotos: Boolean,
        val visibility: String? = null, val attached: List<RecentPhoto> = emptyList(), val federate: Boolean? = null) : AndroidScreen
    data class SignIn(val restoring: Boolean) : AndroidScreen
    data class Call(val caller: String, val incoming: Boolean, val active: Boolean = false,
        val muted: Boolean = false, val speaker: Boolean = false, val avatar: ImageBitmap? = null,
        val hasUser: Boolean = false, val minimize: Boolean = false, val pictureInPicture: Boolean = false,
        val status: String? = null) : AndroidScreen
}

internal sealed interface AndroidUiAction {
    data class Navigate(val page: TopPage) : AndroidUiAction
    data class OpenNotification(val path: String) : AndroidUiAction
    data object RetryNotifications : AndroidUiAction
    data object LoadMoreNotifications : AndroidUiAction
    data class ActOnEntry(val item: TimelineItem, val operation: EntryOperation, val value: String = "", val reactionImageId: String? = null, val quotedMedia: Boolean = false) : AndroidUiAction
    data class RespondFollow(val actor: String, val activity: String, val accept: Boolean) : AndroidUiAction
    data class RemoveFile(val id: String) : AndroidUiAction
    data class SelectInput(val range: TextRange) : AndroidUiAction
    data class Mention(val name: String) : AndroidUiAction
    data class ViewedNotification(val id: String) : AndroidUiAction
    data class OpenChannel(val id: String) : AndroidUiAction
    data class OpenChannelSettings(val id: String) : AndroidUiAction
    data class OpenPost(val id: String) : AndroidUiAction
    data object RetryPost : AndroidUiAction
    data object RetryProfile : AndroidUiAction
    data object RetryProfileAccount : AndroidUiAction
    data object RetryTimeline : AndroidUiAction
    data class OpenProfile(val author: zoo.Row) : AndroidUiAction
    data class ReplyToPost(val id: String) : AndroidUiAction
    data class OpenThread(val id: String) : AndroidUiAction
    data class OpenSetting(val id: String) : AndroidUiAction
    data class Toggle(val id: String, val value: Boolean) : AndroidUiAction
    data class TextChanged(val text: String) : AndroidUiAction
    data class UseInputTool(val tool: InputTool) : AndroidUiAction
    data class Search(val query: String) : AndroidUiAction
    data class SearchMessages(val text: String) : AndroidUiAction
    data object SubmitSearch : AndroidUiAction
    data class SelectSearchType(val type: zoo.GlobalSearchType) : AndroidUiAction
    data class SelectSearchFilter(val field: SearchFilterField, val ids: List<String>) : AndroidUiAction
    data class ToggleSearchFilter(val flag: SearchFilterFlag) : AndroidUiAction
    data class OpenSearchResult(val key: String) : AndroidUiAction
    data class OpenSearchMedia(val key: String, val revealedSensitive: Boolean = false) : AndroidUiAction
    data object LoadOlderResults : AndroidUiAction
    data object RetrySearch : AndroidUiAction
    data class ChannelNameChanged(val name: String) : AndroidUiAction
    data class ChannelDescriptionChanged(val description: String) : AndroidUiAction
    data object SubmitChannel : AndroidUiAction
    data class SelectResult(val id: String) : AndroidUiAction
    data class SelectPhoto(val id: String) : AndroidUiAction
    data object SystemPhotos : AndroidUiAction
    data object SignIn : AndroidUiAction
    data object MinimizeCall : AndroidUiAction
    data object FloatCall : AndroidUiAction
    data object Answer : AndroidUiAction
    data object HangUp : AndroidUiAction
    data object Browse : AndroidUiAction
    data object ComposePost : AndroidUiAction
    data object CreateChannel : AndroidUiAction
    data object CreateDm : AndroidUiAction
    data object LoadOlderPosts : AndroidUiAction
    /** チャンネル（開いているスレッドがあれば、その返信）の、次の古い 1 ページ。 */
    data object LoadOlderMessages : AndroidUiAction
    data class SelectTimeline(val id: String) : AndroidUiAction
    data class SelectPostVisibility(val id: String) : AndroidUiAction
    data object StartCall : AndroidUiAction
    data object MuteCall : AndroidUiAction
    data object SpeakerCall : AndroidUiAction
    data object Back : AndroidUiAction
}

internal data class AndroidInteractionSettings(
    val swipeReply: Boolean = true,
    val haptics: Boolean = true,
    val reducedMotion: Boolean = false,
)

private val SilentHaptics = object : HapticFeedback {
    override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) = Unit
}

@Composable
internal fun AndroidScreens(
    screen: AndroidScreen,
    dark: Boolean,
    foreground: Boolean,
    dispatch: (AndroidUiAction) -> Unit,
    language: String? = null,
    interaction: AndroidInteractionSettings = AndroidInteractionSettings(),
    remoteAudioLevel: () -> Float = { 0f },
    loadNotificationAvatar: suspend (String) -> ImageBitmap? = { null },
    loadNotificationReactions: suspend (InboxReactionContext) -> List<InboxReactionEmoji> = { emptyList() },
    pageState: SaveableStateHolder = rememberSaveableStateHolder(),
    photo: @Composable (RecentPhoto, Modifier) -> Unit,
) {
    val systemHaptics = LocalHapticFeedback.current
    pageState.SaveableStateProvider("screen-history") {
        var recentRoutes by rememberSaveable { mutableStateOf(arrayListOf<String>()) }
        val route = screen.routeKey()
        LaunchedEffect(pageState, route) {
            val updated = recentRoutes.filterNot { it == route } + route
            val retained = updated.takeLast(32)
            updated.dropLast(retained.size).forEach(pageState::removeState)
            recentRoutes = ArrayList(retained)
        }
    }
    CompositionLocalProvider(LocalHapticFeedback provides if (interaction.haptics) systemHaptics else SilentHaptics) {
        AnimatedContent(
            targetState = screen,
            contentKey = { it.routeKey() },
            transitionSpec = {
                val opening = initialState is AndroidScreen.Conversation && targetState is AndroidScreen.Thread
                val closing = initialState is AndroidScreen.Thread && targetState is AndroidScreen.Conversation
                when {
                    interaction.reducedMotion -> EnterTransition.None togetherWith ExitTransition.None
                    opening -> slideInHorizontally(tween(240)) { it } togetherWith
                        slideOutHorizontally(tween(240)) { -it / 4 }
                    closing -> slideInHorizontally(tween(240)) { -it / 4 } togetherWith
                        slideOutHorizontally(tween(240)) { it }
                    else -> EnterTransition.None togetherWith ExitTransition.None
                }
            },
            label = "Android screen",
        ) { visibleScreen ->
            pageState.SaveableStateProvider(visibleScreen.routeKey()) {
                RenderAndroidScreen(visibleScreen, dark, foreground && visibleScreen == screen,
                    dispatch, language, interaction, remoteAudioLevel, loadNotificationAvatar, loadNotificationReactions, photo)
            }
        }
    }
}

private fun AndroidScreen.routeKey(): String = when (this) {
    is AndroidScreen.Home -> "home:$timeline"
    is AndroidScreen.Conversation -> "channel:$id"
    is AndroidScreen.OpeningConversation -> key
    is AndroidScreen.Thread -> "thread:${root.id}"
    is AndroidScreen.Post -> "post:${root.id}"
    is AndroidScreen.Profile -> if (postsPending && posts.isEmpty()) "profile-loading:$id" else "profile:$id"
    is AndroidScreen.ChannelSettings -> "channel-settings:$id"
    is AndroidScreen.SettingsDetail -> "settings-detail:$title"
    is AndroidScreen.Directory -> "directory:$title:${scopeId.orEmpty()}"
    else -> this::class.simpleName.orEmpty()
}

@Composable
private fun RenderAndroidScreen(
    screen: AndroidScreen,
    dark: Boolean,
    foreground: Boolean,
    dispatch: (AndroidUiAction) -> Unit,
    language: String?,
    interaction: AndroidInteractionSettings,
    remoteAudioLevel: () -> Float,
    loadNotificationAvatar: suspend (String) -> ImageBitmap?,
    loadNotificationReactions: suspend (InboxReactionContext) -> List<InboxReactionEmoji>,
    photo: @Composable (RecentPhoto, Modifier) -> Unit,
) {
    val top = when (screen) {
        is AndroidScreen.Home -> TopPage.Home
        is AndroidScreen.Channels -> TopPage.Channels
        is AndroidScreen.Search -> TopPage.Search
        is AndroidScreen.Notifications -> TopPage.Notifications
        is AndroidScreen.Bubble -> TopPage.Bubble
        is AndroidScreen.Settings -> TopPage.Settings
        else -> null
    }
    val back = { dispatch(AndroidUiAction.Back) }
    val openSetting = { id: String -> dispatch(AndroidUiAction.OpenSetting(id)) }
    val toggle = { id: String, value: Boolean -> dispatch(AndroidUiAction.Toggle(id, value)) }
    AndroidShell(top, dark, { dispatch(AndroidUiAction.Navigate(it)) }, language) { colors ->
        when (screen) {
            is AndroidScreen.Home -> HomePage(screen, colors, dispatch)
            is AndroidScreen.Channels -> ChannelPage(screen, colors, dispatch)
            is AndroidScreen.Notifications -> NotificationPage(screen.items, foreground, colors,
                nextCursor = screen.nextCursor, loadAvatar = loadNotificationAvatar, loadReactions = loadNotificationReactions,
                onViewed = { dispatch(AndroidUiAction.ViewedNotification(it)) },
                onOpen = { dispatch(AndroidUiAction.OpenNotification(it)) },
                loadMore = { dispatch(AndroidUiAction.LoadMoreNotifications) },
                loadFailed = screen.loadFailed, retry = { dispatch(AndroidUiAction.RetryNotifications) },
                respond = { actor, activity, accept -> dispatch(AndroidUiAction.RespondFollow(actor, activity, accept)) },
            )
            is AndroidScreen.Bubble -> {
                PageHeader(androidLabel("bubbleChat.title", "Bubble"), colors) {
                    if (screen.participants.isNotEmpty()) UiIconButton(UiIcon.Bubble, "参加者", colors.foreground,
                        { dispatch(AndroidUiAction.OpenSetting("bubble-participants")) })
                }
                Text("入力中の内容がリアルタイムに表示されます", Modifier.padding(16.dp),
                    color = colors.secondary, fontSize = 13.sp)
                // 入力途中の URL で行き先を取得しないよう、Bubble（Web と同じく素の表示）ではアプリ内リンクを扱わない。
                CompositionLocalProvider(LocalChannelLinks provides null) {
                    TimelineWithInput(screen.entries, screen.input, colors, dispatch, canResize = false)
                }
            }
            is AndroidScreen.Settings -> SettingsPage(screen.groups, colors, openSetting, toggle)
            is AndroidScreen.Conversation -> {
                CollapsingHeader(header = {
                    PageHeader(screen.name, colors, screen.description, onBack = back,
                        onOpen = { dispatch(AndroidUiAction.OpenChannelSettings(screen.id)) }) {
                        if (screen.canCall) UiIconButton(UiIcon.Phone, "通話を開始", colors.foreground,
                            { dispatch(AndroidUiAction.StartCall) })
                    }
                }) { headerHeight, headerVisible ->
                    screen.retrySync?.let { retry ->
                        FeedLoadError(androidLabel("common.errors.network", "通信エラー"), colors, retry)
                    }
                    TimelineWithInput(screen.entries, screen.input, colors, dispatch, swipeReply = interaction.swipeReply, allowThreads = true, focusMessage = screen.focusMessage, focusRequest = screen.focusRequest, hideIdleHandle = true, historyReady = screen.historyReady,
                        unreadPositionPending = screen.unreadPositionPending,
                        onUnreadPositioned = screen.onUnreadPositioned,
                        onReadThrough = screen.onReadThrough.takeIf { foreground },
                        topContentPadding = headerHeight, headerOverlayHeight = if (headerVisible) headerHeight else 0.dp,
                        loadEarlier = if (screen.loadOlder) { { dispatch(AndroidUiAction.LoadOlderMessages) } } else null)
                }
            }
            is AndroidScreen.Thread -> {
                PageHeader("スレッド", colors, onBack = back,
                    onOpen = { dispatch(AndroidUiAction.OpenSetting("thread-settings")) })
                Text("# ${screen.channelName}", Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    color = colors.secondary, fontSize = 13.sp)
                TimelineWithInput(listOf(screen.root) + screen.replies, screen.input, colors, dispatch,
                    focusMessage = screen.focusMessage, focusRequest = screen.focusRequest, hideIdleHandle = true, historyReady = screen.historyReady,
                    loadEarlier = if (screen.loadOlder) { { dispatch(AndroidUiAction.LoadOlderMessages) } } else null)
            }
            is AndroidScreen.OpeningConversation -> {
                PageHeader(screen.title, colors, onBack = back)
                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                    when {
                        !screen.online -> Text(androidLabel("offline.bannerTitle", "オフラインです"), color = colors.secondary)
                        screen.failed -> screen.retry?.let { retry ->
                            TextAction(androidLabel("offline.retry", "再試行"), colors, click = retry)
                        }
                        else -> CircularProgressIndicator(color = colors.accent)
                    }
                }
            }
            is AndroidScreen.PostLoading -> {
                PageHeader("投稿", colors, onBack = back)
                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                    when {
                        !screen.online -> Text(androidLabel("offline.bannerTitle", "オフラインです"), color = colors.secondary)
                        screen.failed -> TextAction(androidLabel("offline.retry", "再試行"), colors) {
                            dispatch(AndroidUiAction.RetryPost)
                        }
                        else -> CircularProgressIndicator(color = colors.accent)
                    }
                }
            }
            is AndroidScreen.Post -> {
                PageHeader("投稿", colors, onBack = back)
                if (screen.loadFailed && screen.online) FeedLoadError(androidLabel("common.errors.network", "通信エラー"), colors) {
                    dispatch(AndroidUiAction.RetryPost)
                }
                if (screen.loading && screen.online && !screen.loadFailed)
                    LinearProgressIndicator(Modifier.fillMaxWidth(), color = colors.accent)
                TimelineWithInput(listOf(screen.root) + screen.replies, screen.input, colors, dispatch, posts = true, canResize = false)
            }
            is AndroidScreen.EditEntry -> {
                PageHeader("編集", colors, onBack = back)
                ConversationInput(screen.input, colors, { dispatch(AndroidUiAction.TextChanged(it)) },
                    { dispatch(AndroidUiAction.UseInputTool(it)) }, Modifier.weight(1f),
                    onSelection = { dispatch(AndroidUiAction.SelectInput(it)) }, canResize = false)
            }
            is AndroidScreen.SettingsDetail -> SettingsPage(screen.groups, colors, openSetting, toggle, screen.title, back)
            is AndroidScreen.ChannelSettings -> SettingsPage(screen.groups, colors, openSetting, toggle, screen.channel, back)
            is AndroidScreen.EditChannel -> ChannelEditorPage(screen, colors, dispatch)
            is AndroidScreen.Directory -> DirectoryPage(
                screen.title, screen.query, screen.results, colors, back,
                { dispatch(AndroidUiAction.Search(it)) }, { dispatch(AndroidUiAction.SelectResult(it)) },
                screen.submit, screen.canSubmit, screen.encryption,
                { dispatch(AndroidUiAction.OpenSetting("submit-dm")) },
                { dispatch(AndroidUiAction.OpenSetting("encrypted-dm")) },
            )
            is AndroidScreen.Profile -> {
                PageHeader("プロフィール", colors, onBack = back, paintBackground = false) {
                    if (screen.originalAvailable) UiIconButton(UiIcon.External, "公開プロフィールを開く", colors.foreground,
                        { dispatch(AndroidUiAction.OpenSetting("profile-original")) })
                }
                Timeline(screen.posts, colors, Modifier.weight(1f), dispatch, posts = true, header = {
                    Column {
                        ProfileHeader(screen, colors) { dispatch(AndroidUiAction.OpenSetting("profile-follow")) }
                        if (screen.accountLoadFailed && screen.online) {
                            FeedLoadError(androidLabel("common.errors.network", "通信エラー"), colors) {
                                dispatch(AndroidUiAction.RetryProfileAccount)
                            }
                        }
                        if (screen.postsPending) {
                            Box(Modifier.fillMaxWidth().padding(12.dp), contentAlignment = Alignment.Center) {
                                when {
                                    !screen.online -> Text(androidLabel("offline.bannerTitle", "オフラインです"), color = colors.secondary)
                                    screen.postsLoadFailed -> TextAction(androidLabel("offline.retry", "再試行"), colors) {
                                        dispatch(AndroidUiAction.RetryProfile)
                                    }
                                    else -> CircularProgressIndicator(color = colors.accent)
                                }
                            }
                        }
                    }
                })
            }
            is AndroidScreen.NewPost -> NewPostPage(screen, colors, dispatch, photo)
            is AndroidScreen.Search -> SearchPage(screen, colors, dispatch)
            is AndroidScreen.SignIn -> BrandPage(
                "Zoo", if (screen.restoring) "" else "ブラウザーで認証を完了すると、Zooに戻ります。",
                colors, loading = screen.restoring,
            ) {
                if (screen.restoring) {
                    TextAction("ログイン画面を開く", colors, icon = UiIcon.SignIn, style = TextActionStyle.Plain, click = back)
                } else TextAction("ログイン", colors, icon = UiIcon.SignIn) { dispatch(AndroidUiAction.SignIn) }
            }
            is AndroidScreen.Call -> CallPage(screen, colors, foreground && !interaction.reducedMotion,
                remoteAudioLevel, dispatch)
        }
    }
}

@Composable
private fun HomePage(screen: AndroidScreen.Home, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    CollapsingHeader(forceVisible = screen.posts.isEmpty(), header = {
        Column(Modifier.background(colors.background)) {
            PageHeader(androidLabel("home.title", "ホーム"), colors) {
                UiIconButton(UiIcon.Search, "ユーザーを検索", colors.foreground,
                    { dispatch(AndroidUiAction.OpenSetting("people")) })
            }
            if (screen.timeline != null) Row(Modifier.fillMaxWidth()) {
                for ((id, title) in listOf("home" to "ホーム", "local" to "ローカル", "social" to "ソーシャル")) {
                    TextAction(androidLabel("home.timelineLabels.$id", title), colors, style = if (id == screen.timeline) TextActionStyle.Neutral else TextActionStyle.Plain,
                        modifier = Modifier.weight(1f)) { dispatch(AndroidUiAction.SelectTimeline(id)) }
                }
            }
        }
    }) { headerHeight, headerVisible ->
        Box(Modifier.fillMaxSize()) {
            Timeline(
                screen.posts, colors, Modifier.fillMaxSize(), dispatch, posts = true,
                topContentPadding = headerHeight,
                headerOverlayHeight = if (headerVisible) headerHeight else 0.dp,
                loadOlder = if (screen.loadOlder) { { dispatch(AndroidUiAction.LoadOlderPosts) } } else null,
                header = if (screen.posts.isNotEmpty() && screen.loadFailed && screen.online) {
                    { TimelineLoadError(colors) { dispatch(AndroidUiAction.RetryTimeline) } }
                } else null,
            )
            if (screen.posts.isEmpty()) {
                Column(Modifier.align(Alignment.Center).padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    when {
                        !screen.online -> Text(androidLabel("offline.bannerTitle", "オフラインです"), color = colors.secondary)
                        screen.loading -> CircularProgressIndicator(color = colors.accent)
                        screen.loadFailed -> TimelineLoadError(colors) { dispatch(AndroidUiAction.RetryTimeline) }
                    }
                }
            }
            Box(Modifier.align(Alignment.BottomEnd).padding(16.dp)) {
                UiIconButton(UiIcon.Create, "新しい投稿", Color.Black,
                    action = { dispatch(AndroidUiAction.ComposePost) }, fill = colors.accent)
            }
        }
    }
}

@Composable
private fun TimelineLoadError(colors: AndroidColors, retry: () -> Unit) {
    FeedLoadError(androidLabel("home.loadError", "タイムラインを取得できませんでした。"), colors, retry)
}

@Composable
private fun FeedLoadError(message: String, colors: AndroidColors, retry: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(message, color = colors.secondary)
        TextAction(androidLabel("offline.retry", "再試行"), colors, click = retry)
    }
}

/** The header's ＋: find a channel to join, or create one. */
@Composable
private fun ChannelMenuButton(colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        UiIconButton(UiIcon.Add, "チャンネルを探す・作成", colors.foreground, { expanded = true })
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }, containerColor = colors.background) {
            listOf(
                Triple("チャンネルを探す", UiIcon.Search, AndroidUiAction.Browse),
                Triple("チャンネルを作成", UiIcon.Add, AndroidUiAction.CreateChannel),
            ).forEach { (label, icon, action) ->
                DropdownMenuItem(
                    text = { Text(label, color = colors.foreground) },
                    leadingIcon = { UiGlyph(icon, colors.secondary, AndroidControlSize.inlineIcon) },
                    onClick = { expanded = false; dispatch(action) },
                )
            }
        }
    }
}

@Composable
private fun ChannelPage(screen: AndroidScreen.Channels, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    PageHeader(androidLabel("channels.title", "チャンネル"), colors) { ChannelMenuButton(colors, dispatch) }
    val scroll = rememberLazyListState()
    val visibleScroll = if (screen.sections.any { it.channels.isNotEmpty() }) scroll else rememberLazyListState()
    LazyColumn(state = visibleScroll, contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)) {
        screen.sections.forEach { section ->
            item {
                Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(section.label, Modifier.weight(1f), color = colors.secondary, fontSize = 13.sp,
                        fontWeight = AndroidWeights.label)
                    val dm = section.label == "ダイレクトメッセージ"
                    UiIconButton(
                        UiIcon.Add, if (dm) "DMを作成" else "チャンネルを作成", colors.secondary,
                        { dispatch(if (dm) AndroidUiAction.CreateDm else AndroidUiAction.CreateChannel) },
                        iconSize = AndroidControlSize.inlineIcon,
                    )
                }
            }
            items(section.channels, key = { it.id }) { channel ->
                Row(
                    Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
                        .clickable(role = Role.Button) { dispatch(AndroidUiAction.OpenChannel(channel.id)) }
                        .semantics { if (channel.unread) stateDescription = "未読" },
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    UiGlyph(UiIcon.Channels, colors.secondary, iconSize = AndroidControlSize.inlineIcon)
                    Text(
                        channel.title,
                        Modifier.weight(1f),
                        color = colors.foreground,
                        fontWeight = when {
                            channel.isUserName -> AndroidWeights.userName
                            channel.unread -> AndroidWeights.unreadChannel
                            else -> AndroidWeights.body
                        },
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@Composable
internal fun ColumnScope.TimelineWithInput(
    entries: List<TimelineItem>,
    input: InputState,
    colors: AndroidColors,
    dispatch: (AndroidUiAction) -> Unit,
    swipeReply: Boolean = false,
    allowThreads: Boolean = false,
    focusMessage: String? = null,
    focusRequest: String? = focusMessage,
    posts: Boolean = false,
    hideIdleHandle: Boolean = false,
    canResize: Boolean = true,
    historyReady: Boolean = true,
    loadEarlier: (() -> Unit)? = null,
    topContentPadding: Dp = 0.dp,
    headerOverlayHeight: Dp = 0.dp,
    unreadPositionPending: Boolean = false,
    onUnreadPositioned: (() -> Unit)? = null,
    onReadThrough: ((String?) -> Unit)? = null,
    imeInsets: WindowInsets = WindowInsets.ime.exclude(WindowInsets.systemBars),
) {
    val scroll = rememberLazyListState()
    val density = LocalDensity.current
    val expanded by rememberUpdatedState(input.expanded)
    var resizing by remember { mutableStateOf(false) }
    val overlayInput = input.expanded || resizing
    var compactInputHeight by remember { mutableIntStateOf(0) }
    var positioned by rememberSaveable { mutableStateOf(false) }
    // スレッドは、親が先頭にあるので、最初から先頭に見える。利用者がスクロールするまでは、古い返信を読まない。
    var drags by remember { mutableIntStateOf(0) }
    // ドラッグだけでなく、フリングやアクセシビリティ操作のスクロールも、1 回の操作として数える。
    LaunchedEffect(scroll) {
        snapshotFlow { scroll.isScrollInProgress }.collect { if (it) drags++ }
    }
    var servedDrags by remember { mutableIntStateOf(0) }
    // 最後に要求したときの行数。行数が変わらないまま Effect が再起動したとき（取得の失敗で loadEarlier が
    // 偽から真に戻ったとき）は、利用者がもう一度スクロールするまで、同じページを要求しない。
    var requestedAtSize by remember { mutableIntStateOf(-1) }
    val atTop by remember { derivedStateOf { scroll.firstVisibleItemIndex == 0 } }
    val requestEarlier by rememberUpdatedState(loadEarlier)
    // 先頭に来たら、古い 1 ページを取る。末尾へ初期位置を合わせる前に呼ぶと、開いた直後に全履歴を読んでしまう。
    // チャンネルは、ページが増える（行数が変わる）か、新しくドラッグしたときだけ要求する。成功すると行数が増え、
    // まだ先頭にいれば続けて取る。失敗では行数が変わらないので、ドラッグを待つ。
    // スレッドは、親が先頭に残るので、ページが増えても先頭のまま。スクロール 1 回につき 1 ページだけ取る。
    LaunchedEffect(atTop, positioned, drags, loadEarlier != null, entries.size.takeIf { allowThreads }) {
        if (!atTop || requestEarlier == null) return@LaunchedEffect
        val newDrag = drags > servedDrags
        if (allowThreads) {
            if (!positioned || !(newDrag || entries.size != requestedAtSize)) return@LaunchedEffect
            requestedAtSize = entries.size
        } else if (!newDrag) return@LaunchedEffect
        servedDrags = drags
        requestEarlier?.invoke()
    }
    var openedTarget by rememberSaveable { mutableStateOf<String?>(null) }
    var seenFocusRequest by rememberSaveable { mutableStateOf<String?>(null) }
    var followedSend by rememberSaveable { mutableStateOf<String?>(null) }
    var anchor by rememberSaveable { mutableStateOf<String?>(null) }
    val restoredAnchor = remember { anchor }
    val restoredOffset = remember { scroll.firstVisibleItemScrollOffset }
    var restorePending by remember { mutableStateOf(restoredAnchor != null) }
    var followEnd by rememberSaveable { mutableStateOf(false) }
    val focusedIndex = entries.indexOfFirst { it.id == focusMessage }
    val restoredIndex = if (restorePending && !followEnd) entries.indexOfFirst { it.entryKey == restoredAnchor } else -1
    val unreadIndex = entries.indexOfFirst { it.unreadDivider != null }
    val seekUnread = onUnreadPositioned != null
    LaunchedEffect(input.expanded, input.lastSend, focusRequest, focusedIndex, entries.isNotEmpty(),
        restoredIndex, historyReady, unreadPositionPending, unreadIndex, seekUnread) {
        if (seenFocusRequest != focusRequest) {
            seenFocusRequest = focusRequest
            if (focusRequest != null) {
                followedSend = input.lastSend
                followEnd = false
            } else {
                openedTarget = null
            }
        }
        if (input.expanded || entries.isEmpty()) return@LaunchedEffect
        when {
            input.lastSend != null && input.lastSend != followedSend -> {
                followedSend = input.lastSend
                openedTarget = focusRequest
                restorePending = false
                followEnd = true
                positioned = true
                scroll.requestScrollToItem(entries.lastIndex)
                onUnreadPositioned?.invoke()
            }
            focusedIndex >= 0 && openedTarget != focusRequest -> {
                restorePending = false
                followEnd = false
                scroll.requestScrollToItem(focusedIndex)
                openedTarget = focusRequest
                positioned = true
                onUnreadPositioned?.invoke()
            }
            focusMessage == null && !unreadPositionPending && unreadIndex >= 0 &&
                (seekUnread || (!positioned && !restorePending)) -> {
                restorePending = false
                followEnd = false
                scroll.scrollToItem(unreadIndex, -with(density) { headerOverlayHeight.roundToPx() })
                followEnd = !scroll.canScrollForward
                positioned = true
                onUnreadPositioned?.invoke()
            }
            restoredIndex >= 0 -> {
                scroll.requestScrollToItem(restoredIndex, restoredOffset)
                restorePending = false
            }
            restorePending && !followEnd && historyReady -> {
                restorePending = false
                followEnd = allowThreads
                positioned = true
                scroll.requestScrollToItem(if (allowThreads) entries.lastIndex else 0)
            }
            focusMessage == null && allowThreads && !positioned && !unreadPositionPending -> {
                scroll.scrollToItem(entries.lastIndex)
                followEnd = true
                positioned = true
            }
        }
        if (seekUnread && !unreadPositionPending && (unreadIndex < 0 || (focusRequest != null && openedTarget == focusRequest))) {
            onUnreadPositioned()
        }
    }
    val latestEntry by rememberUpdatedState(entries.lastOrNull())
    val reportReadThrough by rememberUpdatedState(onReadThrough)
    val canReportRead = onReadThrough != null && historyReady && !unreadPositionPending && !seekUnread && !overlayInput
    LaunchedEffect(scroll, canReportRead, focusRequest) {
        if (!canReportRead) return@LaunchedEffect
        val report = reportReadThrough ?: return@LaunchedEffect
        try {
            snapshotFlow {
                val entry = latestEntry
                val last = scroll.layoutInfo.visibleItemsInfo.lastOrNull()
                entry?.id?.takeIf { positioned && !restorePending && !scroll.canScrollForward &&
                    !scroll.isScrollInProgress && (focusRequest == null || openedTarget == focusRequest) &&
                    last?.key == entry.entryKey }
            }.collect { report(it) }
        } finally {
            report(null)
        }
    }
    FollowHistoryDuringIme(scroll, entries, imeInsets, input.expanded, followEnd, focusRequest,
        focusRequest != null && openedTarget != focusRequest)

    // Viewport, row height and item count can all change without user scrolling.
    // Only follow those changes while the reader is at the end or has just sent a message.
    LaunchedEffect(scroll) {
        var wasScrolling = false
        snapshotFlow { Triple(scroll.isScrollInProgress, scroll.layoutInfo, followEnd to (expanded || resizing)) }
            .collect { (scrolling, layout, tracking) ->
                if (tracking.second) return@collect
                if (scrolling) {
                    wasScrolling = true
                    restorePending = false
                } else {
                    if (wasScrolling) {
                        followEnd = !scroll.canScrollForward
                        wasScrolling = false
                    }
                    if (layout.viewportSize.height > 0 && layout.totalItemsCount > 0) {
                        if (followEnd && scroll.canScrollForward) {
                            val lastIndex = layout.totalItemsCount - 1
                            val offset = layout.visibleItemsInfo.lastOrNull()?.takeIf { it.index == lastIndex }?.size ?: 0
                            scroll.requestScrollToItem(lastIndex, offset)
                        } else if (!restorePending || followEnd) {
                            anchor = layout.visibleItemsInfo.firstOrNull { it.index == scroll.firstVisibleItemIndex }?.key as? String
                        }
                    }
                }
            }
    }

    // During the gesture the editor covers the list. Its compact height still
    // determines the list viewport, so changing the editor height cannot scroll it.
    Layout(modifier = Modifier.weight(1f).fillMaxWidth(), content = {
        BoxWithConstraints(
            if (input.expanded) Modifier
                .focusProperties { onEnter = { cancelFocusChange() } }
                .focusGroup()
                .clearAndSetSemantics { hideFromAccessibility() }
            else Modifier,
        ) {
            if (maxHeight > 0.dp && entries.isNotEmpty() && (!input.expanded || resizing)) {
                Timeline(entries, colors, Modifier.fillMaxSize().testTag("conversation-timeline"), dispatch, posts = posts,
                    swipeReply = swipeReply, allowThreads = allowThreads, scroll = scroll, topContentPadding = topContentPadding,
                    headerOverlayHeight = headerOverlayHeight)
            }
        }
        Box(Modifier.onSizeChanged { if (!overlayInput) compactInputHeight = it.height }
            .then(if (input.expanded) Modifier.pointerInput(Unit) { detectTapGestures {} } else Modifier)) {
            ConversationInput(input, colors, { dispatch(AndroidUiAction.TextChanged(it)) },
                { dispatch(AndroidUiAction.UseInputTool(it)) },
                onMention = { dispatch(AndroidUiAction.Mention(it)) },
                onSelection = { dispatch(AndroidUiAction.SelectInput(it)) },
                onRemoveFile = { dispatch(AndroidUiAction.RemoveFile(it)) },
                hideIdleHandle = hideIdleHandle, canResize = canResize,
                onResizeChanged = { resizing = it })
        }
    }) { children, constraints ->
        val editor = children[1].measure(
            constraints.copy(minHeight = 0, maxHeight = (constraints.maxHeight - headerOverlayHeight.roundToPx()).coerceAtLeast(0)),
        )
        val reserved = if (overlayInput) compactInputHeight else editor.height
        val list = children[0].measure(Constraints.fixed(constraints.maxWidth, (constraints.maxHeight - reserved).coerceAtLeast(0)))
        layout(constraints.maxWidth, constraints.maxHeight) {
            list.placeRelative(0, 0)
            editor.placeRelative(0, constraints.maxHeight - editor.height)
        }
    }
}

private data class ImeHistoryAnchor(val itemKey: String, val index: Int, val offset: Int, val inset: Int)

private val ImeHistoryAnchorSaver = listSaver<ImeHistoryAnchor?, Any>(
    save = { anchor -> anchor?.let { listOf(it.itemKey, it.index, it.offset, it.inset) }.orEmpty() },
    restore = { fields -> ImeHistoryAnchor(fields[0] as String, fields[1] as Int, fields[2] as Int, fields[3] as Int) },
)

@Composable
private fun FollowHistoryDuringIme(
    scroll: LazyListState,
    entries: List<TimelineItem>,
    imeInsets: WindowInsets,
    expanded: Boolean,
    followEnd: Boolean,
    focusRequest: String?,
    focusPending: Boolean,
) {
    val density = LocalDensity.current
    val insets by rememberUpdatedState(imeInsets)
    val currentEntries by rememberUpdatedState(entries)
    val editorExpanded by rememberUpdatedState(expanded)
    val followingEnd by rememberUpdatedState(followEnd)
    var anchor by rememberSaveable(focusRequest, stateSaver = ImeHistoryAnchorSaver) {
        mutableStateOf<ImeHistoryAnchor?>(null)
    }
    LaunchedEffect(scroll, density, focusRequest, focusPending) {
        if (focusPending) {
            anchor = null
            return@LaunchedEffect
        }
        var previousInset = insets.getBottom(density)
        snapshotFlow {
            Triple(insets.getBottom(density), scroll.isScrollInProgress, editorExpanded) to
                scroll.layoutInfo.viewportSize.height
        }.collect { (ime, viewportHeight) ->
            val (inset, scrolling, fullEditor) = ime
            val opening = inset > previousInset
            val originalInset = previousInset
            previousInset = inset
            if (scrolling || followingEnd) {
                anchor = null
                return@collect
            }
            if (anchor == null && opening && (fullEditor || viewportHeight > 0)) {
                val index = scroll.firstVisibleItemIndex
                val key = scroll.layoutInfo.visibleItemsInfo.firstOrNull { it.index == index }?.key as? String
                if (key != null) {
                    anchor = ImeHistoryAnchor(key, index, scroll.firstVisibleItemScrollOffset, originalInset)
                }
            }
            if (fullEditor || viewportHeight <= 0) return@collect
            val current = anchor ?: return@collect
            val index = if (currentEntries.getOrNull(current.index)?.entryKey == current.itemKey) current.index
                else currentEntries.indexOfFirst { it.entryKey == current.itemKey }
            if (index >= 0) {
                // Use an absolute position so LazyColumn's own end clamp cannot be counted twice.
                scroll.requestScrollToItem(index, current.offset + (inset - current.inset).coerceAtLeast(0))
            }
            if (index < 0 || inset <= current.inset) anchor = null
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Timeline(
    entries: List<TimelineItem>, colors: AndroidColors, modifier: Modifier,
    dispatch: (AndroidUiAction) -> Unit, posts: Boolean = false,
    swipeReply: Boolean = false,
    allowThreads: Boolean = false,
    loadOlder: (() -> Unit)? = null,
    header: (@Composable () -> Unit)? = null,
    scroll: LazyListState = rememberLazyListState(),
    topContentPadding: Dp = 0.dp,
    headerOverlayHeight: Dp = 0.dp,
) {
    if (entries.isEmpty() && header == null) return
    var menuId by remember { mutableStateOf<String?>(null) }
    val lastVisible = scroll.layoutInfo.visibleItemsInfo.lastOrNull()?.key
    val lastEntry = entries.lastOrNull()?.entryKey
    val requestOlder by rememberUpdatedState(loadOlder)
    LaunchedEffect(lastVisible, lastEntry, loadOlder != null) {
        if (lastEntry != null && lastVisible == lastEntry) requestOlder?.invoke()
    }
    WithHeaderBringIntoView(headerOverlayHeight) { inheritedBringIntoView ->
        LazyColumn(modifier, state = scroll, contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = topContentPadding)) {
            if (header != null) item { header() }
            items(entries, key = { it.entryKey }) { entry ->
                CompositionLocalProvider(LocalBringIntoViewSpec provides inheritedBringIntoView) {
                    entry.dateDivider?.let { ConversationDivider(it, colors) }
                    entry.unreadDivider?.let { ConversationDivider(it, colors, unread = true) }
                    Column(
                        Modifier.fillMaxWidth().testTag("timeline-entry-${entry.id}")
                            .then(if (posts) Modifier.combinedClickable(
                                onClick = { dispatch(AndroidUiAction.OpenPost(entry.id)) },
                                onLongClick = { menuId = entry.id },
                            ) else conversationGestures(
                                open = if (allowThreads) ({ dispatch(AndroidUiAction.OpenThread(entry.id)) }) else null,
                                menu = { menuId = entry.id },
                                swipeReply = swipeReply,
                            ))
                            .padding(top = 12.dp, bottom = if (posts) 4.dp else 12.dp),
                    ) {
                        entry.divider?.let { EncryptionDivider(it, colors) }
                        entry.repostedBy?.let { RepostAttribution(it, colors) }
                        val openProfile = entry.postAuthor.takeIf { posts }?.let { author ->
                            { dispatch(AndroidUiAction.OpenProfile(author)) }
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(if (openProfile == null) 12.dp else 0.dp)) {
                            TimelineAvatar(entry.author, entry.avatar, colors, openProfile = openProfile)
                            Column(Modifier.weight(1f)) {
                                if (posts && entry.postHeader != null) {
                                    PostHeader(entry.author, entry.postHeader, colors, openProfile)
                                } else Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    Text(entry.author, color = colors.foreground, fontWeight = AndroidWeights.entryAuthor, fontSize = entryTextSize)
                                    Text(entry.time, color = colors.secondary, fontSize = 12.sp)
                                }
                                if (entry.marks.isNotEmpty()) EntryMarks(entry.marks, colors)
                                entry.notice?.let {
                                    Text(it, Modifier.testTag("entry-notice-${entry.id}"), color = colors.secondary,
                                        fontSize = entryTextSize, fontStyle = androidx.compose.ui.text.font.FontStyle.Italic)
                                }
                                if (!entry.richContent.hidesBody && entry.body.isNotBlank()) {
                                    EntryBody(entry.body, colors.foreground, entryTextSize, entry.emoji)
                                }
                                EntryBlocks(entry.richContent.blocks, colors, entryTextSize, entry.emoji)
                                entry.attachments.forEach { (id, name) ->
                                    Text(name, Modifier.clickable { dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.Attachment, id)) }
                                        .heightIn(min = 48.dp).padding(vertical = 12.dp), color = colors.foreground)
                                }
                                TimelineMediaGroup(entry.media, colors, apub = posts) { media ->
                                    dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.Media, media.key))
                                }
                                entry.quote?.let { quote ->
                                    TimelineQuoteCard(quote, colors,
                                        open = { dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.Quote)) },
                                        openProfile = quote.postAuthor.takeIf { posts }?.let { author ->
                                            { dispatch(AndroidUiAction.OpenProfile(author)) }
                                        },
                                        openMedia = { dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.Media, it.key, quotedMedia = true)) })
                                }
                                entry.channelLinks.forEach { url -> ChannelLinkCard(url, colors, menu = { menuId = entry.id }) }
                                EntryCards(listOfNotNull(entry.linkCard), colors)
                                entry.link?.takeIf { entry.linkCard == null }?.let { url ->
                                    Text(url,
                                        Modifier.clickable { dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.Link, url)) }
                                            .heightIn(min = 48.dp).padding(vertical = 12.dp),
                                        color = colors.secondary, maxLines = 2, overflow = TextOverflow.Ellipsis)
                                }
                                if (entry.reactions.isNotEmpty()) {
                                    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                        entry.reactions.forEach { reaction ->
                                            TimelineReactionChip(reaction, colors) {
                                                dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.React, reaction.value, reaction.imageId))
                                            }
                                        }
                                    }
                                }
                                if (posts) PostActions(entry, colors, dispatch, onMore = { menuId = entry.id })
                            }
                        }
                    }
                }
            }
        }
    }
    entries.firstOrNull { it.id == menuId }?.let { entry ->
        EntryActionSheet(
            entry = entry,
            colors = colors,
            posts = posts,
            dismiss = { menuId = null },
            action = { operation, value ->
                menuId = null
                dispatch(AndroidUiAction.ActOnEntry(entry, operation, value))
            },
            open = if (posts || allowThreads) ({
                menuId = null
                dispatch(if (posts) AndroidUiAction.OpenPost(entry.id) else AndroidUiAction.OpenThread(entry.id))
            }) else null,
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun WithHeaderBringIntoView(headerOverlayHeight: Dp, content: @Composable (BringIntoViewSpec) -> Unit) {
    val inheritedBringIntoView = LocalBringIntoViewSpec.current
    val overlayPixels = with(LocalDensity.current) { headerOverlayHeight.toPx() }
    val bringIntoView = remember(inheritedBringIntoView, overlayPixels) {
        object : BringIntoViewSpec by inheritedBringIntoView {
            override fun calculateScrollDistance(offset: Float, size: Float, containerSize: Float): Float {
                val covered = overlayPixels.coerceAtMost(containerSize)
                if (offset >= 0 && offset < covered && size >= containerSize - covered) {
                    return offset - covered
                }
                return inheritedBringIntoView.calculateScrollDistance(offset - covered, size, containerSize - covered)
            }
        }
    }
    CompositionLocalProvider(LocalBringIntoViewSpec provides bringIntoView) { content(inheritedBringIntoView) }
}

@Composable
private fun PostActions(
    entry: TimelineItem,
    colors: AndroidColors,
    dispatch: (AndroidUiAction) -> Unit,
    onMore: () -> Unit,
) {
    val actionWidth = Modifier.width(AndroidControlSize.postToolbarIcon)
    Row(
        Modifier.fillMaxWidth().height(AndroidControlSize.postToolbarHeight),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        UiIconButton(UiIcon.Bubble, "返信", colors.secondary,
            { dispatch(AndroidUiAction.ReplyToPost(entry.id)) }, enabled = entry.postHeader?.audience != null,
            iconSize = AndroidControlSize.postToolbarIcon, modifier = actionWidth)
        UiIconButton(UiIcon.Repost, if (entry.reposted) "再投稿を取り消す" else "再投稿",
            if (entry.reposted) colors.foreground else colors.secondary.copy(alpha = if (entry.canRepost) 1f else .38f),
            { dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.Repost)) },
            enabled = entry.canRepost, iconSize = AndroidControlSize.postToolbarIcon, modifier = actionWidth)
        UiIconButton(UiIcon.Emoji, "リアクションを追加",
            colors.secondary.copy(alpha = if (entry.canReact) 1f else .38f),
            { dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.React)) },
            enabled = entry.canReact, iconSize = AndroidControlSize.postToolbarIcon, modifier = actionWidth)
        UiIconButton(UiIcon.More, "その他の投稿操作", colors.secondary, onMore, iconSize = AndroidControlSize.postToolbarIcon, modifier = actionWidth)
    }
}

@Composable
private fun conversationGestures(open: (() -> Unit)?, menu: () -> Unit, swipeReply: Boolean): Modifier {
    val currentOpen by rememberUpdatedState(open)
    val currentMenu by rememberUpdatedState(menu)
    val haptics = LocalHapticFeedback.current
    var drag by remember { mutableFloatStateOf(0f) }
    return Modifier.offset { IntOffset(drag.roundToInt(), 0) }
        .pointerInput(haptics) {
            detectTapGestures(onLongPress = {
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                currentMenu()
            })
        }
        .then(if (!swipeReply || open == null) Modifier else Modifier.pointerInput(haptics) {
            val threshold = 64.dp.toPx()
            val maxDrag = 96.dp.toPx()
            val edge = 8.dp.toPx()
            var reached = false
            var allowed = false
            detectHorizontalDragGestures(
                onDragStart = { start ->
                    drag = 0f
                    reached = false
                    allowed = start.x in edge..(size.width - edge)
                },
                onDragCancel = { drag = 0f },
                onDragEnd = {
                    val openThread = drag <= -threshold
                    drag = 0f
                    if (openThread) currentOpen?.invoke()
                },
            ) { change, dx ->
                if (allowed) {
                    change.consume()
                    drag = (drag + dx).coerceIn(-maxDrag, 0f)
                    if (!reached && drag <= -threshold) {
                        reached = true
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    }
                }
            }
        })
        .semantics {
            customActions = buildList {
                if (open != null) add(CustomAccessibilityAction("スレッドを開く") {
                    currentOpen?.invoke()
                    true
                })
                add(CustomAccessibilityAction("メッセージの操作") {
                    currentMenu()
                    true
                })
            }
        }
}

@Composable
private fun ConversationDivider(label: String, colors: AndroidColors, unread: Boolean = false) {
    val line = if (unread) colors.unread else colors.inputBoundary
    Row(
        Modifier.fillMaxWidth().padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        HorizontalDivider(Modifier.weight(1f), color = line)
        Text(label, color = if (unread) colors.unread else colors.secondary,
            fontSize = 12.sp, fontWeight = AndroidWeights.emphasis)
        HorizontalDivider(if (unread) Modifier.width(24.dp) else Modifier.weight(1f), color = line)
    }
}

/** 暗号化に切り替えた境目（Web の `MessageList` の区切り）。上は平文、下は暗号化したメッセージ。 */
@Composable
private fun EncryptionDivider(label: String, colors: AndroidColors) {
    Row(
        Modifier.fillMaxWidth().padding(bottom = 8.dp).semantics(mergeDescendants = true) { contentDescription = label },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(Modifier.weight(1f).height(1.dp).background(colors.inputBoundary))
        UiGlyph(UiIcon.Lock, colors.secondary, iconSize = 12.dp)
        Text(label, color = colors.secondary, fontSize = 12.sp, fontWeight = AndroidWeights.label)
        Box(Modifier.width(24.dp).height(1.dp).background(colors.inputBoundary))
    }
}

/** 送信者の確かさや版の警告、暗号化されていない行の印（Web の `MessageRow` のバッジ）。 */
@Composable
private fun EntryMarks(marks: List<String>, colors: AndroidColors) {
    Row(
        Modifier.horizontalScroll(rememberScrollState()).padding(vertical = 2.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        marks.forEach { mark ->
            Row(
                Modifier.background(colors.quietSurface, CircleShape).padding(horizontal = 8.dp, vertical = 2.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                UiGlyph(UiIcon.Lock, colors.secondary, iconSize = 12.dp)
                Text(mark, color = colors.secondary, fontSize = 12.sp)
            }
        }
    }
}
