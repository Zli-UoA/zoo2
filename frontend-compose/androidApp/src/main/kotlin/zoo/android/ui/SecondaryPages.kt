package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.toggleable
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class SelectionItem(
    val id: String,
    val title: String,
    val detail: String? = null,
    val isUserName: Boolean = false,
    val selected: Boolean? = null,
)
internal data class RecentPhoto(val id: String, val label: String)

@Composable
internal fun DirectoryPage(
    title: String, query: String, results: List<SelectionItem>, colors: AndroidColors,
    back: () -> Unit, search: (String) -> Unit, select: (String) -> Unit,
    submit: String?, canSubmit: Boolean, encryption: Boolean?, onSubmit: () -> Unit, onEncryption: () -> Unit,
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
                        Text(item.title, color = colors.foreground, fontSize = 16.sp,
                            fontWeight = if (item.isUserName) AndroidWeights.userName else AndroidWeights.body)
                        item.detail?.let { Text(it, color = colors.secondary, fontSize = 13.sp) }
                    }
                    val selected = item.selected
                    if (selected == null) UiGlyph(UiIcon.Next, colors.secondary, iconSize = AndroidControlSize.inlineIcon)
                    else androidx.compose.material3.Checkbox(selected, onCheckedChange = null)
                }
            }
        }
        if (encryption != null) {
            Row(Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target).clickable(onClick = onEncryption),
                verticalAlignment = Alignment.CenterVertically) {
                Text("エンドツーエンド暗号化", Modifier.weight(1f), color = colors.foreground)
                androidx.compose.material3.Switch(encryption, onCheckedChange = null)
            }
        }
        if (submit != null) Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
            TextAction(submit, colors, canSubmit, icon = UiIcon.Bubble, click = onSubmit)
        }
    }
}

@Composable
internal fun NewPostPage(
    screen: AndroidScreen.NewPost, colors: AndroidColors,
    dispatch: (AndroidUiAction) -> Unit,
    photo: @Composable (RecentPhoto, Modifier) -> Unit,
) {
    val input = screen.input
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
        if (screen.showPhotos) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("最近の写真", Modifier.weight(1f), color = colors.secondary, fontSize = 13.sp)
                TextAction("写真を選択", colors, icon = UiIcon.Photo, style = TextActionStyle.Plain) {
                    dispatch(AndroidUiAction.SystemPhotos)
                }
            }
            LazyRow(
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                items(screen.recent, key = { it.id }) { item ->
                    photo(item, Modifier.size(72.dp).clip(RoundedCornerShape(10.dp))
                        .clickable(role = Role.Button) { dispatch(AndroidUiAction.SelectPhoto(item.id)) })
                }
            }
        }
        if (screen.attached.isNotEmpty()) LazyRow(
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            items(screen.attached, key = { it.id }) { item ->
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    photo(item, Modifier.size(72.dp).clip(RoundedCornerShape(10.dp)))
                    TextAction("削除", colors, icon = UiIcon.Delete, style = TextActionStyle.Plain) { dispatch(AndroidUiAction.RemoveFile(item.id)) }
                }
            }
        }
        screen.federate?.let { enabled ->
            Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp).heightIn(min = 48.dp)
                .clickable { dispatch(AndroidUiAction.OpenSetting("toggle-federation")) },
                verticalAlignment = Alignment.CenterVertically) {
                Text("ActivityPub で配信", Modifier.weight(1f), color = colors.foreground)
                androidx.compose.material3.Switch(enabled, onCheckedChange = null)
            }
        }
        if (screen.visibility != null) Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp)) {
            for ((id, label) in listOf("public" to "公開", "unlisted" to "未収載", "followers" to "フォロワー", "local" to "ローカル")) {
                TextAction(label, colors, style = if (id == screen.visibility) TextActionStyle.Neutral else TextActionStyle.Plain,
                    modifier = Modifier.weight(1f)) { dispatch(AndroidUiAction.SelectPostVisibility(id)) }
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
            val enabled = input.canSend && (input.text.isNotBlank() || screen.attached.isNotEmpty())
            TextAction("投稿", colors, enabled, icon = UiIcon.Send) { dispatch(AndroidUiAction.UseInputTool(InputTool.Send)) }
        }
    }
}

@Composable
internal fun BrandPage(
    title: String, detail: String, colors: AndroidColors, loading: Boolean = false,
    detailWeight: FontWeight = AndroidWeights.body,
    badge: @Composable () -> Unit = { ZooBadge(colors, Modifier.size(88.dp)) },
    actions: @Composable ColumnScope.() -> Unit,
) {
    Column(
        Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(colors.headerLight, colors.background), endY = 680f))
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Spacer(Modifier.weight(2f))
        badge()
        Text(title, color = colors.foreground, fontSize = 32.sp, fontWeight = AndroidWeights.emphasis)
        if (detail.isNotEmpty()) Text(detail, color = colors.secondary, fontSize = 16.sp, fontWeight = detailWeight, textAlign = TextAlign.Center)
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
        val options = listOf(
            Triple("channel-private", "非公開", screen.privateChannel),
            Triple("channel-encrypted", "エンドツーエンド暗号化", screen.encrypted),
            Triple("channel-sensitive", "センシティブな内容を含む", screen.sensitive),
        )
        options.forEach { (id, label, selection) ->
            if (selection != null) Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp)
                    .toggleable(selection, role = Role.Switch) { dispatch(AndroidUiAction.OpenSetting(id)) },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(label, Modifier.weight(1f), color = colors.foreground)
                androidx.compose.material3.Switch(selection, onCheckedChange = null)
            }
        }
        Spacer(Modifier.weight(1f))
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
            TextAction(screen.submitLabel, colors, screen.canSubmit,
                icon = if (screen.submitLabel == "保存") UiIcon.Save else UiIcon.Add) { dispatch(AndroidUiAction.SubmitChannel) }
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
