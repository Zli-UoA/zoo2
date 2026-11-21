package zoo.android.redesign

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import zoo.ZooMark

@Composable
internal fun TopContent(destination: Destination, state: SceneState, palette: Palette, dispatch: (Action) -> Unit) {
    Header(destination.label, palette)
    when (destination) {
        Destination.Settings -> LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
            items(listOf("テーマ", "言語", "操作", "通知", "アプリの更新")) { name ->
                Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable { dispatch(Action.Setting(name)) },
                    verticalAlignment = Alignment.CenterVertically) {
                    Text(name, Modifier.weight(1f), color = palette.ink)
                    Glyph("chevron-right", name, palette) { dispatch(Action.Setting(name)) }
                }
            }
        }
        Destination.Home -> Box(Modifier.fillMaxSize()) {
            Entries(state, state.entries, palette, Modifier.fillMaxSize(), dispatch)
            Box(Modifier.align(Alignment.BottomEnd).padding(16.dp).background(Signal, CircleShape)) {
                Glyph("pencil", "新しい投稿", palette.copy(ink = Color.Black)) { dispatch(Action.NewPost) }
            }
        }
        Destination.Notifications -> Entries(state, state.entries, palette, Modifier.fillMaxSize(), dispatch)
        Destination.Bubble -> {
            Text("入力中の内容がリアルタイムに表示されます", Modifier.padding(16.dp), color = palette.quiet, fontSize = 13.sp)
            Entries(state, state.entries, palette, Modifier.weight(1f), dispatch)
            Editor(state, palette, dispatch)
        }
        Destination.Channels -> Unit
    }
}

@Composable
internal fun Entries(
    state: SceneState,
    entries: List<Entry>,
    palette: Palette,
    modifier: Modifier,
    dispatch: (Action) -> Unit,
) {
    if (state.expanded) return
    LazyColumn(modifier.fillMaxWidth().background(palette.canvas), contentPadding = PaddingValues(horizontal = 16.dp)) {
        items(entries) { entry ->
            Row(
                Modifier.fillMaxWidth().combinedClickable(onLongClick = { dispatch(Action.OpenThread(entry)) }, onClick = {})
                    .padding(vertical = 10.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Box(Modifier.size(32.dp).background(palette.surface, CircleShape), contentAlignment = Alignment.Center) {
                    Text(entry.author.take(1), color = palette.ink, fontSize = 12.sp)
                }
                Column {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(entry.author, color = palette.ink, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                        Text(entry.time, color = palette.quiet, fontSize = 11.sp)
                    }
                    Text(entry.body, color = palette.ink, fontSize = 16.sp, lineHeight = 22.sp)
                }
            }
            HorizontalDivider(color = palette.quiet.copy(alpha = .20f))
        }
    }
}

@Composable
internal fun ColumnScope.Editor(state: SceneState, palette: Palette, dispatch: (Action) -> Unit) {
    Column(Modifier.fillMaxWidth().then(if (state.expanded) Modifier.weight(1f) else Modifier)
        .background(palette.surface)) {
        Box(Modifier.fillMaxWidth().height(24.dp).clickable { dispatch(Action.Expand) }, contentAlignment = Alignment.Center) {
            Box(Modifier.size(28.dp, 3.dp).background(palette.quiet, CircleShape))
        }
        BasicTextField(
            state.draft, { dispatch(Action.Draft(it)) },
            modifier = Modifier.fillMaxWidth().then(if (state.expanded) Modifier.weight(1f) else Modifier.heightIn(min = 40.dp))
                .padding(horizontal = 16.dp, vertical = 6.dp),
            textStyle = TextStyle(color = palette.ink, fontSize = 16.sp, lineHeight = 22.sp),
            decorationBox = { inner ->
                Box {
                    if (state.draft.isEmpty()) Text("メッセージを送信", color = palette.quiet, fontSize = 16.sp)
                    inner()
                }
            },
        )
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Glyph("paperclip", "添付", palette) { dispatch(Action.Attach) }
            Glyph("case-sensitive", "書式", palette) { dispatch(Action.Format) }
            Glyph("smile", "絵文字", palette) { dispatch(Action.Emoji) }
            Spacer(Modifier.weight(1f))
            Box(Modifier.background(if (state.draft.isNotBlank()) Signal else palette.canvas, CircleShape)) {
                Glyph("send", "送信", if (state.draft.isNotBlank()) palette.copy(ink = Color.Black) else palette) {
                    if (state.draft.isNotBlank()) dispatch(Action.Send)
                }
            }
        }
    }
}

@Composable
internal fun SignIn(state: Scene.SignIn, palette: Palette, dispatch: (Action) -> Unit) {
    BrandScreen("Zoo", if (state.restoring) "" else "ブラウザーで認証すると Zoo に戻ります。", palette) {
        if (state.restoring) CircularProgressIndicator(Modifier.size(24.dp), color = Signal, strokeWidth = 2.dp)
        else Primary("ログイン") { dispatch(Action.SignIn) }
    }
}

@Composable
internal fun Call(state: Scene.Call, palette: Palette, dispatch: (Action) -> Unit) {
    BrandScreen(if (state.incoming) "着信中" else "通話中", state.caller, palette) {
        if (state.incoming) Primary("応答") { dispatch(Action.Answer) }
        Text("通話を終了", Modifier.background(palette.surface, CircleShape).clickable { dispatch(Action.HangUp) }
            .padding(horizontal = 24.dp, vertical = 14.dp), color = palette.ink, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
private fun BrandScreen(title: String, description: String, palette: Palette, actions: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize().background(
        androidx.compose.ui.graphics.Brush.verticalGradient(
            0f to palette.light, .55f to palette.canvas,
        ),
    ).padding(horizontal = 20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Spacer(Modifier.weight(2f))
        ZooMark(84.dp)
        Spacer(Modifier.height(24.dp))
        Text(title, color = palette.ink, fontSize = 30.sp, fontWeight = FontWeight.Bold)
        if (description.isNotEmpty()) Text(description, Modifier.padding(top = 8.dp), color = palette.quiet, textAlign = TextAlign.Center)
        Spacer(Modifier.weight(2f))
        Column(Modifier.padding(bottom = 24.dp), horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp), content = actions)
    }
}

@Composable
private fun Primary(label: String, click: () -> Unit) {
    Text(label, Modifier.background(Signal, CircleShape).clickable(onClick = click)
        .padding(horizontal = 24.dp, vertical = 14.dp), color = Color.Black, fontWeight = FontWeight.SemiBold)
}
