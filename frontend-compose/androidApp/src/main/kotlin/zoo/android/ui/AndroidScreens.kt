package zoo.android.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import androidx.compose.ui.unit.sp

internal data class TimelineItem(val id: String, val author: String, val body: String, val time: String)
internal data class ChannelItem(
    val id: String,
    val title: String,
    val unread: Boolean = false,
    val isUserName: Boolean = false,
)
internal data class ChannelSection(val label: String, val channels: List<ChannelItem>)

internal sealed interface AndroidScreen {
    data class Home(val posts: List<TimelineItem>, val timeline: String? = null) : AndroidScreen
    data class Channels(val sections: List<ChannelSection>) : AndroidScreen
    data class Notifications(val items: List<InboxItem>) : AndroidScreen
    data class Bubble(val entries: List<TimelineItem>, val input: InputState) : AndroidScreen
    data class Settings(val groups: List<SettingsGroup>) : AndroidScreen
    data class Conversation(
        val id: String, val name: String, val description: String,
        val entries: List<TimelineItem>, val input: InputState, val canCall: Boolean = false,
        val focusMessage: String? = null,
    ) : AndroidScreen
    data class Thread(
        val channelName: String, val root: TimelineItem, val replies: List<TimelineItem>, val input: InputState,
        val focusMessage: String? = null,
    ) : AndroidScreen
    data class Post(val root: TimelineItem, val replies: List<TimelineItem>, val input: InputState) : AndroidScreen
    data class ChannelSettings(val channel: String, val groups: List<SettingsGroup>) : AndroidScreen
    data class SettingsDetail(val title: String, val groups: List<SettingsGroup>) : AndroidScreen
    data class EditChannel(
        val title: String,
        val name: String,
        val description: String,
        val submitLabel: String,
        val canSubmit: Boolean,
    ) : AndroidScreen
    data class Directory(val title: String, val query: String, val results: List<SelectionItem>) : AndroidScreen
    data class Profile(val name: String, val bio: String, val posts: List<TimelineItem>) : AndroidScreen
    data class NewPost(val input: InputState, val recent: List<RecentPhoto>, val showPhotos: Boolean,
        val visibility: String? = null, val attached: List<RecentPhoto> = emptyList()) : AndroidScreen
    data class SignIn(val restoring: Boolean) : AndroidScreen
    data class Call(val caller: String, val incoming: Boolean, val active: Boolean = false,
        val muted: Boolean = false, val speaker: Boolean = false, val avatar: ImageBitmap? = null,
        val hasUser: Boolean = false) : AndroidScreen
}

internal sealed interface AndroidUiAction {
    data class Navigate(val page: TopPage) : AndroidUiAction
    data class OpenNotification(val path: String) : AndroidUiAction
    data object LoadMoreNotifications : AndroidUiAction
    data class ViewedNotification(val id: String) : AndroidUiAction
    data class OpenChannel(val id: String) : AndroidUiAction
    data class OpenChannelSettings(val id: String) : AndroidUiAction
    data class OpenPost(val id: String) : AndroidUiAction
    data class OpenThread(val id: String) : AndroidUiAction
    data class OpenSetting(val id: String) : AndroidUiAction
    data class Toggle(val id: String, val value: Boolean) : AndroidUiAction
    data class TextChanged(val text: String) : AndroidUiAction
    data class UseInputTool(val tool: InputTool) : AndroidUiAction
    data class Search(val query: String) : AndroidUiAction
    data class ChannelNameChanged(val name: String) : AndroidUiAction
    data class ChannelDescriptionChanged(val description: String) : AndroidUiAction
    data object SubmitChannel : AndroidUiAction
    data class SelectResult(val id: String) : AndroidUiAction
    data class SelectPhoto(val id: String) : AndroidUiAction
    data object SystemPhotos : AndroidUiAction
    data object SignIn : AndroidUiAction
    data object Answer : AndroidUiAction
    data object HangUp : AndroidUiAction
    data object Browse : AndroidUiAction
    data object ComposePost : AndroidUiAction
    data object CreateChannel : AndroidUiAction
    data object CreateDm : AndroidUiAction
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
    photo: @Composable (RecentPhoto, Modifier) -> Unit,
) {
    val systemHaptics = LocalHapticFeedback.current
    val conversationState = rememberSaveableStateHolder()
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
            if (visibleScreen is AndroidScreen.Conversation) {
                conversationState.SaveableStateProvider(visibleScreen.id) {
                    RenderAndroidScreen(visibleScreen, dark, foreground && visibleScreen == screen,
                        dispatch, language, interaction, remoteAudioLevel, photo)
                }
            } else {
                RenderAndroidScreen(visibleScreen, dark, foreground && visibleScreen == screen,
                    dispatch, language, interaction, remoteAudioLevel, photo)
            }
        }
    }
}

private fun AndroidScreen.routeKey(): String = when (this) {
    is AndroidScreen.Conversation -> "channel:$id"
    is AndroidScreen.Thread -> "thread:${root.id}"
    is AndroidScreen.Post -> "post:${root.id}"
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
    photo: @Composable (RecentPhoto, Modifier) -> Unit,
) {
    val top = when (screen) {
        is AndroidScreen.Home -> TopPage.Home
        is AndroidScreen.Channels -> TopPage.Channels
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
                onViewed = { dispatch(AndroidUiAction.ViewedNotification(it)) },
                onOpen = { dispatch(AndroidUiAction.OpenNotification(it)) },
                loadMore = { dispatch(AndroidUiAction.LoadMoreNotifications) },
            )
            is AndroidScreen.Bubble -> {
                PageHeader("Bubble", colors)
                Text("入力中の内容がリアルタイムに表示されます", Modifier.padding(16.dp),
                    color = colors.secondary, fontSize = 13.sp)
                TimelineWithInput(screen.entries, screen.input, colors, dispatch)
            }
            is AndroidScreen.Settings -> SettingsPage(screen.groups, colors, openSetting, toggle)
            is AndroidScreen.Conversation -> {
                CollapsingHeader(header = {
                    PageHeader(screen.name, colors, screen.description, onBack = back,
                        onOpen = { dispatch(AndroidUiAction.OpenChannelSettings(screen.id)) }) {
                        if (screen.canCall) UiIconButton(UiIcon.Phone, "通話を開始", colors.foreground,
                            { dispatch(AndroidUiAction.StartCall) })
                    }
                }) {
                    TimelineWithInput(screen.entries, screen.input, colors, dispatch, swipeReply = interaction.swipeReply, allowThreads = true, focusMessage = screen.focusMessage)
                }
            }
            is AndroidScreen.Thread -> {
                PageHeader("スレッド", colors, onBack = back)
                Text("# ${screen.channelName}", Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    color = colors.secondary, fontSize = 13.sp)
                TimelineWithInput(listOf(screen.root) + screen.replies, screen.input, colors, dispatch,
                    focusMessage = screen.focusMessage)
            }
            is AndroidScreen.Post -> {
                PageHeader("投稿", colors, onBack = back)
                TimelineWithInput(listOf(screen.root) + screen.replies, screen.input, colors, dispatch)
            }
            is AndroidScreen.SettingsDetail -> SettingsPage(screen.groups, colors, openSetting, toggle, screen.title, back)
            is AndroidScreen.ChannelSettings -> SettingsPage(screen.groups, colors, openSetting, toggle, screen.channel, back)
            is AndroidScreen.EditChannel -> ChannelEditorPage(screen, colors, dispatch)
            is AndroidScreen.Directory -> DirectoryPage(
                screen.title, screen.query, screen.results, colors, back,
                { dispatch(AndroidUiAction.Search(it)) }, { dispatch(AndroidUiAction.SelectResult(it)) },
            )
            is AndroidScreen.Profile -> {
                PageHeader(screen.name, colors, onBack = back, titleWeight = AndroidWeights.userName)
                Text(screen.bio, Modifier.padding(16.dp), color = colors.secondary, fontSize = 15.sp)
                Timeline(screen.posts, colors, Modifier.weight(1f), dispatch, posts = true)
            }
            is AndroidScreen.NewPost -> NewPostPage(screen, colors, dispatch, photo)
            is AndroidScreen.SignIn -> BrandPage(
                "Zoo", if (screen.restoring) "" else "ブラウザーで認証を完了すると、Zooに戻ります。",
                colors, loading = screen.restoring,
            ) {
                if (screen.restoring) {
                    TextAction("ログイン画面を開く", colors, style = TextActionStyle.Plain, click = back)
                } else TextAction("ログイン", colors) { dispatch(AndroidUiAction.SignIn) }
            }
            is AndroidScreen.Call -> BrandPage(
                if (screen.incoming) "着信中" else "通話中", screen.caller, colors,
                detailWeight = AndroidWeights.userName,
                badge = {
                    CallAvatar(screen, colors, foreground && !interaction.reducedMotion, remoteAudioLevel)
                },
            ) {
                Column(Modifier.width(IntrinsicSize.Max), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    if (screen.active) Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) {
                        TextAction(if (screen.muted) "ミュートを解除" else "ミュート", colors, style = TextActionStyle.Plain) {
                            dispatch(AndroidUiAction.MuteCall)
                        }
                        TextAction(if (screen.speaker) "スピーカーをオフ" else "スピーカー", colors, style = TextActionStyle.Plain) {
                            dispatch(AndroidUiAction.SpeakerCall)
                        }
                    }

                    if (screen.incoming) {
                        TextAction("応答", colors, modifier = Modifier.fillMaxWidth(),
                            icon = UiIcon.Phone, labelWeight = AndroidWeights.emphasis) {
                            dispatch(AndroidUiAction.Answer)
                        }
                    }
                    TextAction("通話を終了", colors, style = TextActionStyle.Neutral, modifier = Modifier.fillMaxWidth(),
                        icon = UiIcon.PhoneOff, labelWeight = AndroidWeights.emphasis) {
                        dispatch(AndroidUiAction.HangUp)
                    }
                }
            }
        }
    }
}

@Composable
private fun HomePage(screen: AndroidScreen.Home, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    CollapsingHeader(header = {
        Column {
            PageHeader("ホーム", colors)
            if (screen.timeline != null) Row(Modifier.fillMaxWidth()) {
                for ((id, title) in listOf("home" to "フォロー中", "local" to "ローカル", "social" to "連合")) {
                    TextAction(title, colors, style = if (id == screen.timeline) TextActionStyle.Neutral else TextActionStyle.Plain,
                        modifier = Modifier.weight(1f)) { dispatch(AndroidUiAction.SelectTimeline(id)) }
                }
            }
        }
    }) {
        Box(Modifier.fillMaxSize()) {
            Timeline(screen.posts, colors, Modifier.fillMaxSize(), dispatch, posts = true)
            Box(Modifier.align(Alignment.BottomEnd).padding(16.dp)) {
                UiIconButton(UiIcon.Create, "新しい投稿", Color.Black,
                    action = { dispatch(AndroidUiAction.ComposePost) }, fill = colors.accent)
            }
        }
    }
}

@Composable
private fun ChannelPage(screen: AndroidScreen.Channels, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    PageHeader("チャンネル", colors)
    LazyColumn(contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)) {
        item {
            Text("チャンネルを探す",
                Modifier.fillMaxWidth().background(colors.quietSurface, CircleShape)
                    .heightIn(min = AndroidControlSize.target)
                    .clickable(role = Role.Button) { dispatch(AndroidUiAction.Browse) }.padding(14.dp),
                color = colors.foreground, fontSize = 15.sp)
        }
        screen.sections.forEach { section ->
            item {
                Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(section.label, Modifier.weight(1f), color = colors.secondary, fontSize = 13.sp,
                        fontWeight = AndroidWeights.label)
                    val dm = section.label == "ダイレクトメッセージ"
                    UiIconButton(UiIcon.Create, if (dm) "DMを作成" else "チャンネルを作成", colors.secondary,
                        { dispatch(if (dm) AndroidUiAction.CreateDm else AndroidUiAction.CreateChannel) })
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
                    UiGlyph(UiIcon.Channels, colors.secondary)
                    Text(
                        channel.title,
                        Modifier.weight(1f),
                        color = colors.foreground,
                        fontWeight = when {
                            channel.isUserName -> AndroidWeights.userName
                            channel.unread -> AndroidWeights.emphasis
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
private fun ColumnScope.TimelineWithInput(
    entries: List<TimelineItem>,
    input: InputState,
    colors: AndroidColors,
    dispatch: (AndroidUiAction) -> Unit,
    swipeReply: Boolean = false,
    allowThreads: Boolean = false,
    focusMessage: String? = null,
) {
    if (!input.expanded) Timeline(entries, colors, Modifier.weight(1f), dispatch,
        swipeReply = swipeReply, allowThreads = allowThreads, focusMessage = focusMessage)
    ConversationInput(input, colors, { dispatch(AndroidUiAction.TextChanged(it)) },
        { dispatch(AndroidUiAction.UseInputTool(it)) }, if (input.expanded) Modifier.weight(1f) else Modifier)
}

@Composable
private fun Timeline(
    entries: List<TimelineItem>, colors: AndroidColors, modifier: Modifier,
    dispatch: (AndroidUiAction) -> Unit, posts: Boolean = false,
    swipeReply: Boolean = false,
    allowThreads: Boolean = false,
    focusMessage: String? = null,
) {
    var menuId by remember { mutableStateOf<String?>(null) }
    val scroll = rememberLazyListState()
    val focusedIndex = entries.indexOfFirst { it.id == focusMessage }
    var openedTarget by rememberSaveable { mutableStateOf<String?>(null) }
    LaunchedEffect(focusMessage, focusedIndex) {
        if (focusMessage == null) openedTarget = null
        else if (focusedIndex >= 0 && openedTarget != focusMessage) {
            scroll.scrollToItem(focusedIndex)
            openedTarget = focusMessage
        }
    }
    LazyColumn(modifier, state = scroll, contentPadding = PaddingValues(horizontal = 16.dp)) {
        items(entries, key = { it.id }) { entry ->
            Row(
                Modifier.fillMaxWidth().testTag("timeline-entry-${entry.id}")
                    .then(if (posts) Modifier.combinedClickable(
                        onClick = { dispatch(AndroidUiAction.OpenPost(entry.id)) },
                        onLongClick = { menuId = entry.id },
                    ) else conversationGestures(
                        open = if (allowThreads) ({ dispatch(AndroidUiAction.OpenThread(entry.id)) }) else null,
                        menu = { menuId = entry.id },
                        swipeReply = swipeReply,
                    ))
                    .padding(vertical = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Box(Modifier.size(36.dp).background(colors.background, CircleShape), contentAlignment = Alignment.Center) {
                    Text(entry.author.take(1), color = colors.secondary, fontSize = 13.sp)
                }
                Column(Modifier.weight(1f)) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(entry.author, color = colors.foreground, fontWeight = AndroidWeights.userName, fontSize = 14.sp)
                        Text(entry.time, color = colors.secondary, fontSize = 12.sp)
                    }
                    Text(entry.body, color = colors.foreground, fontSize = 17.sp, lineHeight = 25.sp)
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
            open = if (posts || allowThreads) ({
                menuId = null
                dispatch(if (posts) AndroidUiAction.OpenPost(entry.id) else AndroidUiAction.OpenThread(entry.id))
            }) else null,
        )
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
