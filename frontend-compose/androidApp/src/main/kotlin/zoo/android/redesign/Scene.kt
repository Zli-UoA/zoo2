package zoo.android.redesign

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import zoo.Icon
import zoo.ZooMark

internal enum class Destination(val label: String, val icon: String) {
    Home("ホーム", "house"),
    Channels("チャンネル", "hash"),
    Notifications("通知", "bell"),
    Bubble("Bubble", "message-circle"),
    Settings("設定", "settings"),
}

internal data class Channel(val id: String, val name: String, val icon: String = "hash", val unread: Boolean = false)
internal data class Entry(val author: String, val body: String, val time: String = "")

internal sealed interface Scene {
    data class Top(val destination: Destination) : Scene
    data class Conversation(val name: String, val description: String, val entries: List<Entry>) : Scene
    data class Thread(val channel: String, val parent: Entry, val replies: List<Entry>) : Scene
    data class Detail(val title: String, val entries: List<Entry>) : Scene
    data class SignIn(val restoring: Boolean = false) : Scene
    data class Call(val caller: String, val incoming: Boolean) : Scene
}

internal data class SceneState(
    val scene: Scene,
    val channels: List<Channel> = emptyList(),
    val dms: List<Channel> = emptyList(),
    val entries: List<Entry> = emptyList(),
    val draft: String = "",
    val expanded: Boolean = false,
)

internal sealed interface Action {
    data class Navigate(val destination: Destination) : Action
    data class OpenChannel(val id: String) : Action
    data class OpenThread(val entry: Entry) : Action
    data class Draft(val value: String) : Action
    data class Setting(val key: String) : Action
    data object Back : Action
    data object Browse : Action
    data object NewPost : Action
    data object ChannelSettings : Action
    data object Expand : Action
    data object Send : Action
    data object Attach : Action
    data object Format : Action
    data object Emoji : Action
    data object SignIn : Action
    data object Answer : Action
    data object HangUp : Action
}

internal data class Palette(val canvas: Color, val ink: Color, val quiet: Color, val surface: Color, val light: Color)
internal val Signal = Color(0xffffd500)

@Composable
internal fun ZooAndroidScene(state: SceneState, dark: Boolean, dispatch: (Action) -> Unit) {
    val palette = if (dark) Palette(Color.Black, Color.White, Color(0xffafada2), Color(0xff1b1a17), Color(0xff2c2405))
        else Palette(Color.White, Color(0xff171612), Color(0xff646058), Color(0xfff7f5ee), Color(0xfffff1b8))
    MaterialTheme {
        Column(Modifier.fillMaxSize().background(palette.canvas).safeDrawingPadding().imePadding()) {
            when (val scene = state.scene) {
                is Scene.Top -> {
                    Column(Modifier.weight(1f)) {
                        when (scene.destination) {
                            Destination.Channels -> ChannelIndex(state, palette, dispatch)
                            else -> TopContent(scene.destination, state, palette, dispatch)
                        }
                    }
                    Navigation(scene.destination, palette, dispatch)
                }
                is Scene.Conversation -> {
                    Header(scene.name, palette, scene.description, { dispatch(Action.Back) }) { dispatch(Action.ChannelSettings) }
                    Entries(state, scene.entries, palette, Modifier.weight(1f), dispatch)
                    Editor(state, palette, dispatch)
                }
                is Scene.Thread -> {
                    Header("スレッド", palette, back = { dispatch(Action.Back) })
                    Text("# ${scene.channel}", Modifier.padding(horizontal = 16.dp), color = palette.quiet, fontSize = 13.sp)
                    Entries(state, listOf(scene.parent) + scene.replies, palette, Modifier.weight(1f), dispatch)
                    Editor(state, palette, dispatch)
                }
                is Scene.Detail -> {
                    Header(scene.title, palette, back = { dispatch(Action.Back) })
                    Entries(state, scene.entries, palette, Modifier.weight(1f), dispatch)
                }
                is Scene.SignIn -> SignIn(scene, palette, dispatch)
                is Scene.Call -> Call(scene, palette, dispatch)
            }
        }
    }
}

@Composable
internal fun Header(
    title: String,
    palette: Palette,
    description: String? = null,
    back: (() -> Unit)? = null,
    open: (() -> Unit)? = null,
) {
    Row(
        Modifier.fillMaxWidth().background(Brush.verticalGradient(listOf(palette.light, palette.canvas)))
            .heightIn(min = 56.dp).padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (back == null) ZooMark(24.dp)
        else Glyph("arrow-left", "戻る", palette, back)
        Column(Modifier.weight(1f).then(if (open != null) Modifier.clickable(role = Role.Button, onClick = open) else Modifier)
            .padding(start = 8.dp)) {
            Text(title, color = palette.ink, fontSize = if (description == null) 20.sp else 16.sp, fontWeight = FontWeight.SemiBold)
            description?.let { Text(it, color = palette.quiet, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
        if (open != null) Glyph("chevron-right", "チャンネルの設定", palette, open)
    }
}

@Composable
internal fun Glyph(icon: String, label: String, palette: Palette, click: () -> Unit) {
    Box(Modifier.size(48.dp).clickable(role = Role.Button, onClick = click)
        .semantics { contentDescription = label }, contentAlignment = Alignment.Center) {
        Icon(icon, modifier = Modifier.size(22.dp), color = palette.ink)
    }
}

@Composable
private fun Navigation(current: Destination, palette: Palette, dispatch: (Action) -> Unit) {
    Row(Modifier.fillMaxWidth().background(palette.surface).heightIn(min = 60.dp)) {
        for (destination in Destination.entries) {
            Column(Modifier.weight(1f).heightIn(min = 60.dp)
                .clickable(role = Role.Tab) { dispatch(Action.Navigate(destination)) }
                .semantics { selected = destination == current }, horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center) {
                Box(Modifier.background(if (destination == current) palette.light else Color.Transparent, CircleShape)
                    .padding(horizontal = 12.dp, vertical = 3.dp)) {
                    Icon(destination.icon, modifier = Modifier.size(20.dp), color = palette.ink)
                }
                Text(destination.label, fontSize = 11.sp, color = palette.ink)
            }
        }
    }
}

@Composable
private fun ChannelIndex(state: SceneState, palette: Palette, dispatch: (Action) -> Unit) {
    Header("チャンネル", palette)
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
        Row(Modifier.fillMaxWidth().padding(vertical = 8.dp).background(palette.surface, CircleShape)
            .clickable(role = Role.Button) { dispatch(Action.Browse) }.heightIn(min = 48.dp).padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon("search", modifier = Modifier.size(20.dp), color = palette.quiet)
            Text("チャンネルを探す", color = palette.ink)
        }
        ChannelRows("参加中", state.channels, palette, dispatch)
        ChannelRows("ダイレクトメッセージ", state.dms, palette, dispatch)
    }
}

@Composable
private fun ChannelRows(label: String, channels: List<Channel>, palette: Palette, dispatch: (Action) -> Unit) {
    Text(label, Modifier.padding(top = 12.dp, bottom = 4.dp), color = palette.quiet, fontSize = 13.sp)
    for (channel in channels) {
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) { dispatch(Action.OpenChannel(channel.id)) }
            .padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(channel.icon, modifier = Modifier.size(20.dp), color = palette.quiet)
            Text(channel.name, Modifier.weight(1f), color = palette.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (channel.unread) Text("未読", Modifier.background(palette.light, CircleShape).padding(horizontal = 8.dp),
                color = palette.ink, fontSize = 12.sp)
        }
    }
}
