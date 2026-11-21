package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class SelectionItem(val id: String, val title: String, val detail: String? = null)
internal data class RecentPhoto(val id: String, val label: String)

@Composable
internal fun DirectoryPage(
    title: String, query: String, results: List<SelectionItem>, colors: AndroidColors,
    back: () -> Unit, search: (String) -> Unit, select: (String) -> Unit,
) {
    PageHeader(title, colors, onBack = back)
    Column(Modifier.fillMaxSize().padding(16.dp)) {
        BasicTextField(
            query, search, modifier = Modifier.fillMaxWidth().background(colors.inputSurface, CircleShape).padding(14.dp),
            singleLine = true, textStyle = LocalTextStyle.current.copy(color = colors.inputText, fontSize = 16.sp),
            cursorBrush = SolidColor(colors.inputText),
            decorationBox = { field ->
                Box {
                    if (query.isEmpty()) Text("検索", color = colors.inputHint)
                    field()
                }
            },
        )
        LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(vertical = 8.dp)) {
            items(results, key = { it.id }) { item ->
                Row(
                    Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
                        .clickable(role = Role.Button) { select(item.id) },
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                        Text(item.title, color = colors.foreground, fontSize = 16.sp)
                        item.detail?.let { Text(it, color = colors.secondary, fontSize = 13.sp) }
                    }
                    UiGlyph(UiIcon.Next, colors.secondary)
                }
            }
        }
    }
}

@Composable
internal fun NewPostPage(
    input: InputState, recent: List<RecentPhoto>, showPhotos: Boolean, colors: AndroidColors,
    dispatch: (AndroidUiAction) -> Unit,
    photo: @Composable (RecentPhoto, Modifier) -> Unit,
) {
    PageHeader("新しい投稿", colors, onBack = { dispatch(AndroidUiAction.Back) })
    Column(Modifier.fillMaxSize().imePadding()) {
        BasicTextField(
            input.text, { dispatch(AndroidUiAction.TextChanged(it)) },
            Modifier.weight(1f).fillMaxWidth().padding(16.dp),
            textStyle = LocalTextStyle.current.copy(color = colors.foreground, fontSize = 17.sp, lineHeight = 25.sp),
            cursorBrush = SolidColor(colors.inputText),
            decorationBox = { field ->
                Box {
                    if (input.text.isEmpty()) Text("いま何してる？", color = colors.inputHint)
                    field()
                }
            },
        )
        if (showPhotos) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("最近の写真", Modifier.weight(1f), color = colors.secondary, fontSize = 13.sp)
                TextAction("写真を選択", colors, style = TextActionStyle.Plain) {
                    dispatch(AndroidUiAction.SystemPhotos)
                }
            }
            LazyRow(
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                items(recent, key = { it.id }) { item ->
                    photo(item, Modifier.size(72.dp).clip(RoundedCornerShape(10.dp))
                        .clickable(role = Role.Button) { dispatch(AndroidUiAction.SelectPhoto(item.id)) })
                }
            }
        }
        Row(
            Modifier.fillMaxWidth().background(colors.inputSurface).testTag("post-actions")
                .padding(AndroidControlSize.postActionInset),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            UiIconButton(UiIcon.Attach, "写真", colors.inputText, { dispatch(AndroidUiAction.UseInputTool(InputTool.Attach)) })
            UiIconButton(UiIcon.Emoji, "絵文字", colors.inputText, { dispatch(AndroidUiAction.UseInputTool(InputTool.Emoji)) })
            Spacer(Modifier.weight(1f))
            val enabled = input.canSend && input.text.isNotBlank()
            TextAction("投稿", colors, enabled) { dispatch(AndroidUiAction.UseInputTool(InputTool.Send)) }
        }
    }
}

@Composable
internal fun BrandPage(
    title: String, detail: String, colors: AndroidColors, loading: Boolean = false,
    actions: @Composable ColumnScope.() -> Unit,
) {
    Column(
        Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(colors.headerLight, colors.background), endY = 680f))
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Spacer(Modifier.weight(2f))
        ZooBadge(colors, Modifier.size(88.dp))
        Text(title, color = colors.foreground, fontSize = 32.sp, fontWeight = AndroidWeights.emphasis)
        if (detail.isNotEmpty()) Text(detail, color = colors.secondary, fontSize = 16.sp, textAlign = TextAlign.Center)
        if (loading) CircularProgressIndicator(Modifier.size(22.dp), color = colors.accent, strokeWidth = 2.dp)
        Spacer(Modifier.weight(3f))
        actions()
    }
}

@Composable
internal fun ChannelEditorPage(
    screen: AndroidScreen.EditChannel,
    colors: AndroidColors,
    dispatch: (AndroidUiAction) -> Unit,
) {
    PageHeader(screen.title, colors, onBack = { dispatch(AndroidUiAction.Back) })
    Column(Modifier.fillMaxSize().imePadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ChannelField("チャンネル名", screen.name, colors, singleLine = true) {
            dispatch(AndroidUiAction.ChannelNameChanged(it))
        }
        ChannelField("説明", screen.description, colors, singleLine = false) {
            dispatch(AndroidUiAction.ChannelDescriptionChanged(it))
        }
        Spacer(Modifier.weight(1f))
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
            TextAction(screen.submitLabel, colors, screen.canSubmit) { dispatch(AndroidUiAction.SubmitChannel) }
        }
    }
}

@Composable
private fun ChannelField(
    label: String,
    value: String,
    colors: AndroidColors,
    singleLine: Boolean,
    change: (String) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(label, color = colors.secondary, fontSize = 13.sp)
        BasicTextField(
            value, change,
            Modifier.fillMaxWidth().background(colors.inputSurface, RoundedCornerShape(10.dp)).padding(12.dp),
            singleLine = singleLine, minLines = if (singleLine) 1 else 3,
            textStyle = LocalTextStyle.current.copy(color = colors.inputText, fontSize = 16.sp),
            cursorBrush = SolidColor(colors.inputText),
        )
    }
}
