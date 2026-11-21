package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class TimelineItem(val id: String, val author: String, val body: String, val time: String)
internal data class ChannelItem(val id: String, val title: String, val unread: Boolean = false)
internal data class ChannelSection(val label: String, val channels: List<ChannelItem>)

internal sealed interface AndroidScreen {
    data class Home(val posts: List<TimelineItem>) : AndroidScreen
    data class Channels(val sections: List<ChannelSection>) : AndroidScreen
    data class Notifications(val items: List<InboxItem>) : AndroidScreen
    data class Bubble(val entries: List<TimelineItem>, val input: InputState) : AndroidScreen
    data class Settings(val groups: List<SettingsGroup>) : AndroidScreen
    data class Conversation(
        val id: String, val name: String, val description: String,
        val entries: List<TimelineItem>, val input: InputState,
    ) : AndroidScreen
    data class Thread(
        val channelName: String, val root: TimelineItem, val replies: List<TimelineItem>, val input: InputState,
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
    data class NewPost(val input: InputState, val recent: List<RecentPhoto>, val showPhotos: Boolean) : AndroidScreen
    data class SignIn(val restoring: Boolean) : AndroidScreen
    data class Call(val caller: String, val incoming: Boolean) : AndroidScreen
}

internal sealed interface AndroidUiAction {
    data class Navigate(val page: TopPage) : AndroidUiAction
    data class ViewedNotification(val id: String) : AndroidUiAction
    data class OpenChannel(val id: String) : AndroidUiAction
    data class OpenChannelSettings(val id: String) : AndroidUiAction
    data class OpenPost(val id: String) : AndroidUiAction
    data class EntryMenu(val id: String) : AndroidUiAction
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
    data object Back : AndroidUiAction
}

@Composable
internal fun AndroidScreens(
    screen: AndroidScreen,
    dark: Boolean,
    foreground: Boolean,
    dispatch: (AndroidUiAction) -> Unit,
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
    AndroidShell(top, dark, { dispatch(AndroidUiAction.Navigate(it)) }) { colors ->
        when (screen) {
            is AndroidScreen.Home -> HomePage(screen, colors, dispatch)
            is AndroidScreen.Channels -> ChannelPage(screen, colors, dispatch)
            is AndroidScreen.Notifications -> NotificationPage(screen.items, foreground, colors) {
                dispatch(AndroidUiAction.ViewedNotification(it))
            }
            is AndroidScreen.Bubble -> {
                PageHeader("Bubble", colors)
                Text("入力中の内容がリアルタイムに表示されます", Modifier.padding(16.dp),
                    color = colors.secondary, fontSize = 13.sp)
                TimelineWithInput(screen.entries, screen.input, colors, dispatch)
            }
            is AndroidScreen.Settings -> SettingsPage(screen.groups, colors, openSetting, toggle)
            is AndroidScreen.Conversation -> {
                PageHeader(screen.name, colors, screen.description, onBack = back,
                    onOpen = { dispatch(AndroidUiAction.OpenChannelSettings(screen.id)) })
                TimelineWithInput(screen.entries, screen.input, colors, dispatch)
            }
            is AndroidScreen.Thread -> {
                PageHeader("スレッド", colors, onBack = back)
                Text("# ${screen.channelName}", Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    color = colors.secondary, fontSize = 13.sp)
                TimelineWithInput(listOf(screen.root) + screen.replies, screen.input, colors, dispatch)
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
                PageHeader(screen.name, colors, onBack = back)
                Text(screen.bio, Modifier.padding(16.dp), color = colors.secondary, fontSize = 15.sp)
                Timeline(screen.posts, colors, Modifier.weight(1f), dispatch, posts = true)
            }
            is AndroidScreen.NewPost -> NewPostPage(screen.input, screen.recent, screen.showPhotos, colors, dispatch, photo)
            is AndroidScreen.SignIn -> BrandPage(
                "Zoo", if (screen.restoring) "" else "ブラウザーで認証を完了すると、Zooに戻ります。",
                colors, loading = screen.restoring,
            ) {
                if (screen.restoring) {
                    Text("ログイン画面を開く", Modifier.clickable(role = Role.Button, onClick = back).padding(12.dp),
                        color = colors.foreground)
                } else PrimaryAction("ログイン", colors) { dispatch(AndroidUiAction.SignIn) }
            }
            is AndroidScreen.Call -> BrandPage(if (screen.incoming) "着信中" else "通話中", screen.caller, colors) {
                if (screen.incoming) PrimaryAction("応答", colors) { dispatch(AndroidUiAction.Answer) }
                Text("通話を終了", Modifier.clip(CircleShape).background(colors.inputSurface)
                    .clickable(role = Role.Button) { dispatch(AndroidUiAction.HangUp) }
                    .padding(horizontal = 24.dp, vertical = 14.dp), color = colors.foreground)
            }
        }
    }
}

@Composable
private fun HomePage(screen: AndroidScreen.Home, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    PageHeader("ホーム", colors)
    Box(Modifier.fillMaxSize()) {
        Timeline(screen.posts, colors, Modifier.fillMaxSize(), dispatch, posts = true)
        Box(Modifier.align(Alignment.BottomEnd).padding(16.dp).background(colors.accent, CircleShape)) {
            UiIconButton(UiIcon.Create, "新しい投稿", Color.Black, { dispatch(AndroidUiAction.ComposePost) })
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
                    .clickable(role = Role.Button) { dispatch(AndroidUiAction.Browse) }.padding(14.dp),
                color = colors.foreground, fontSize = 15.sp)
        }
        screen.sections.forEach { section ->
            item {
                Text(section.label, Modifier.padding(top = 16.dp, bottom = 4.dp), color = colors.secondary,
                    fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
            }
            items(section.channels, key = { it.id }) { channel ->
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 48.dp)
                        .clickable(role = Role.Button) { dispatch(AndroidUiAction.OpenChannel(channel.id)) },
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    UiGlyph(UiIcon.Channels, colors.secondary)
                    Text(channel.title, Modifier.weight(1f), color = colors.foreground, maxLines = 1,
                        overflow = TextOverflow.Ellipsis)
                    if (channel.unread) Text("未読",
                        Modifier.background(colors.headerLight, CircleShape).padding(horizontal = 8.dp),
                        color = colors.foreground, fontSize = 12.sp)
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
) {
    if (!input.expanded) Timeline(entries, colors, Modifier.weight(1f), dispatch)
    ConversationInput(input, colors, { dispatch(AndroidUiAction.TextChanged(it)) },
        { dispatch(AndroidUiAction.UseInputTool(it)) }, if (input.expanded) Modifier.weight(1f) else Modifier)
}

@Composable
private fun Timeline(
    entries: List<TimelineItem>, colors: AndroidColors, modifier: Modifier,
    dispatch: (AndroidUiAction) -> Unit, posts: Boolean = false,
) {
    LazyColumn(modifier, contentPadding = PaddingValues(horizontal = 16.dp)) {
        items(entries, key = { it.id }) { entry ->
            Row(
                Modifier.fillMaxWidth()
                    .then(if (posts) Modifier.combinedClickable(
                        onClick = { dispatch(AndroidUiAction.OpenPost(entry.id)) },
                        onLongClick = { dispatch(AndroidUiAction.EntryMenu(entry.id)) },
                    ) else conversationGestures(
                        open = { dispatch(AndroidUiAction.OpenThread(entry.id)) },
                        menu = { dispatch(AndroidUiAction.EntryMenu(entry.id)) },
                    ))
                    .padding(vertical = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Box(Modifier.size(36.dp).background(colors.background, CircleShape), contentAlignment = Alignment.Center) {
                    Text(entry.author.take(1), color = colors.secondary, fontSize = 13.sp)
                }
                Column(Modifier.weight(1f)) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(entry.author, color = colors.foreground, fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
                        Text(entry.time, color = colors.secondary, fontSize = 12.sp)
                    }
                    Text(entry.body, color = colors.foreground, fontSize = 17.sp, lineHeight = 25.sp)
                }
            }
            HorizontalDivider(color = colors.inputBoundary)
        }
    }
}

@Composable
private fun conversationGestures(open: () -> Unit, menu: () -> Unit): Modifier {
    val currentOpen by rememberUpdatedState(open)
    val currentMenu by rememberUpdatedState(menu)
    val haptics = LocalHapticFeedback.current
    return Modifier
        .pointerInput(Unit) { detectTapGestures(onLongPress = { currentMenu() }) }
        .pointerInput(haptics) {
            val threshold = 64.dp.toPx()
            var travel = 0f
            var reached = false
            detectHorizontalDragGestures(
                onDragStart = { travel = 0f; reached = false },
                onDragEnd = { if (travel <= -threshold) currentOpen() },
            ) { change, dx ->
                change.consume()
                travel += dx
                if (!reached && travel <= -threshold) {
                    reached = true
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                }
            }
        }
        .semantics {
            customActions = listOf(
                CustomAccessibilityAction("スレッドを開く") { currentOpen(); true },
                CustomAccessibilityAction("メッセージの操作") { currentMenu(); true },
            )
        }
}
