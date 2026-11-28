package zoo.android.ui

import androidx.compose.foundation.Canvas
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
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
    val unavailable: String? = null,
)
@Composable
internal fun DraftAttachment(name: String, failed: Boolean, colors: AndroidColors,
    retry: () -> Unit, remove: () -> Unit, preview: @Composable (Modifier) -> Unit) {
    Box(Modifier.size(96.dp).clip(RoundedCornerShape(10.dp)).background(colors.inputSurface)) {
        preview(Modifier.fillMaxSize())
        if (failed) {
            Box(Modifier.matchParentSize().background(Color.Black.copy(alpha = 0.38f)))
            UiIconButton(UiIcon.Refresh, "${name}のアップロードを再試行", Color.White, retry,
                fill = Color.Black.copy(alpha = 0.76f), iconSize = 22.dp,
                modifier = Modifier.align(Alignment.Center).size(40.dp))
        }
        UiIconButton(UiIcon.Close, "${name}を添付から外す", Color.White, remove,
            fill = Color.Black.copy(alpha = 0.76f), iconSize = 16.dp,
            modifier = Modifier.align(Alignment.TopEnd).padding(3.dp).size(26.dp))
    }
}

internal data class RecentPhoto(val id: String, val label: String, val failed: Boolean = false, val description: String = "", val sensitive: Boolean = false)

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
                        .clickable(enabled = item.unavailable == null, role = Role.Button) { select(item.id) },
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                        Text(item.title, color = colors.foreground, fontSize = 16.sp,
                            fontWeight = if (item.isUserName) AndroidWeights.userName else AndroidWeights.body)
                        item.detail?.let { Text(it, color = colors.secondary, fontSize = 13.sp) }
                    }
                    val selected = item.selected
                    if (item.unavailable != null) Text(item.unavailable, color = colors.secondary, fontSize = 13.sp)
                    else if (selected == null) UiGlyph(UiIcon.Next, colors.secondary, iconSize = AndroidControlSize.inlineIcon)
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
    var describing by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf<String?>(null) }
    PageHeader("新しい投稿", colors, onBack = { dispatch(AndroidUiAction.Back) }) {
        screen.visibility?.let { PostAudienceButton(it, colors, dispatch) }
        screen.federate?.let { enabled ->
            FederationButton(enabled, colors) { dispatch(AndroidUiAction.OpenSetting("toggle-federation")) }
        }
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        var editing by remember { mutableStateOf(TextFieldValue(input.text, input.selection ?: TextRange(input.text.length))) }
        val value = if (editing.text == input.text && (input.selection == null || input.selection == editing.selection)) {
            editing
        } else TextFieldValue(input.text, input.selection ?: TextRange(input.text.length))
        BasicTextField(
            value, {
                editing = it
                if (it.text != input.text) dispatch(AndroidUiAction.TextChanged(it.text))
                if (it.selection != input.selection) dispatch(AndroidUiAction.SelectInput(it.selection))
            },
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
        CharacterCount(input.text, 1000, colors)
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
                Column {
                    DraftAttachment(item.label, item.failed, colors,
                        retry = { dispatch(AndroidUiAction.RetryFile(item.id)) },
                        remove = { dispatch(AndroidUiAction.RemoveFile(item.id)) }) { modifier ->
                        photo(item, modifier)
                    }
                    Row {
                        UiIconButton(UiIcon.Create, "${item.label}の説明を編集", colors.secondary,
                            { describing = item.id }, iconSize = 18.dp, modifier = Modifier.size(44.dp, 36.dp))
                        UiIconButton(if (item.sensitive) UiIcon.EyeOff else UiIcon.Eye,
                            item.label + if (item.sensitive) "のセンシティブ指定を解除" else "をセンシティブに指定",
                            colors.secondary, { dispatch(AndroidUiAction.SetPhotoSensitive(item.id, !item.sensitive)) },
                            iconSize = 18.dp, modifier = Modifier.size(44.dp, 36.dp))
                    }
                }
            }
        }
        screen.attached.firstOrNull { it.id == describing }?.let { item ->
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Column(Modifier.weight(1f)) {
                    ChannelField("画像の説明 · ${item.label}", item.description, colors, singleLine = false) { text ->
                        if (text.codePointCount(0, text.length) <= 1000) dispatch(AndroidUiAction.DescribePhoto(item.id, text))
                    }
                }
                TextAction("完了", colors, style = TextActionStyle.Plain) { describing = null }
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
            val enabled = input.canSend && input.text.codePointCount(0, input.text.length) <= 1000 && (input.text.isNotBlank() || screen.attached.isNotEmpty())
            TextAction("投稿", colors, enabled, icon = UiIcon.Send) { dispatch(AndroidUiAction.UseInputTool(InputTool.Send)) }
        }
    }
}

/** Paint immediately; show the current operation only when restoration outlasts a short startup. */
@Composable
internal fun RestorationPage(dark: Boolean, stage: String, cancel: (() -> Unit)?) {
    var showProgress by androidx.compose.runtime.remember { mutableStateOf(false) }
    androidx.compose.runtime.LaunchedEffect(Unit) {
        kotlinx.coroutines.delay(400)
        showProgress = true
    }
    AndroidShell(null, dark, {}, tintedHeader = false) { colors ->
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Column(horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(10.dp)) {
                ZooBadge(colors, Modifier.size(72.dp))
                if (showProgress) {
                    Text(stage, color = colors.secondary, fontSize = 14.sp)
                    if (cancel != null) TextAction("キャンセル", colors, style = TextActionStyle.Plain, click = cancel)
                }
            }
        }
    }
}

@Composable
internal fun BrandPage(
    title: String, detail: String, colors: AndroidColors, loading: Boolean = false,
    detailWeight: FontWeight = AndroidWeights.body,
    compact: Boolean = false,
    badge: @Composable () -> Unit = { ZooBadge(colors, Modifier.size(88.dp)) },
    actions: @Composable ColumnScope.() -> Unit,
) {
    Column(
        Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(colors.headerLight, colors.background), endY = 680f))
            .then(if (compact) Modifier.verticalScroll(rememberScrollState()).padding(8.dp) else Modifier.padding(24.dp)),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (!compact) {
            Spacer(Modifier.weight(2f))
            badge()
        }
        Text(title, color = colors.foreground, fontSize = 32.sp, fontWeight = AndroidWeights.emphasis)
        if (detail.isNotEmpty()) Text(detail, color = colors.secondary, fontSize = 16.sp, fontWeight = detailWeight, textAlign = TextAlign.Center)
        if (loading) CircularProgressIndicator(Modifier.size(22.dp), color = colors.accent, strokeWidth = 2.dp)
        if (!compact) Spacer(Modifier.weight(3f))
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
    Column(Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ChannelField("チャンネル名", screen.name, colors, singleLine = true) {
            dispatch(AndroidUiAction.ChannelNameChanged(it))
        }
        ChannelField("説明", screen.description, colors, singleLine = false) {
            dispatch(AndroidUiAction.ChannelDescriptionChanged(it))
        }
        screen.saveError?.let { Text(it, color = colors.unread, fontSize = 14.sp) }
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
            if (id == "channel-encrypted" && screen.privateChannel != null && screen.submitLabel == "作成") {
                Text(if (selection == true) "公開に切り替えると暗号化は解除されます。" else "暗号化を利用するにはチャンネルを非公開にします。",
                    color = colors.secondary, fontSize = 13.sp)
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


@Composable
private fun PostAudienceButton(visibility: String, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    val audience = PostAudience.fromApi(visibility) ?: return
    var expanded by remember { mutableStateOf(false) }
    Box {
        UiIconButton(audience.icon, "${androidLabel("home.audience", "公開範囲")}：${audience.localizedLabel()}", colors.foreground, { expanded = true })
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }, containerColor = colors.background) {
            for (option in PostAudience.composerOptions) {
                DropdownMenuItem(
                    text = { Text(option.localizedLabel(), color = colors.foreground) },
                    leadingIcon = { UiGlyph(option.icon, colors.secondary, AndroidControlSize.inlineIcon) },
                    trailingIcon = if (option == audience) {
                        { UiGlyph(UiIcon.Check, colors.foreground, AndroidControlSize.inlineIcon) }
                    } else null,
                    onClick = {
                        expanded = false
                        dispatch(AndroidUiAction.SelectPostVisibility(option.apiValue))
                    },
                )
            }
        }
    }
}

@Composable
private fun FederationButton(enabled: Boolean, colors: AndroidColors, onToggle: () -> Unit) {
    Box(
        Modifier.size(AndroidControlSize.target).clip(CircleShape)
            .toggleable(value = enabled, role = Role.Switch, onValueChange = { onToggle() })
            .semantics { contentDescription = "ActivityPub で配信" },
        contentAlignment = Alignment.Center,
    ) {
        // ActivityPub mark by mray, CC0. https://activitypub.rocks/images/ActivityPub-logo.svg
        val left = remember {
            PathParser().parsePathString("M10.91,4.44 0,10.74 0,13.26 8.73,8.22 8.73,18.30 10.91,19.56ZM6.55,12 2.18,14.52 6.55,17.04Z").toPath()
        }
        val right = remember {
            PathParser().parsePathString("M13.09,4.44 24,10.74 24,13.26 13.09,19.56 13.09,17.04 21.82,12 13.09,6.96ZM13.09,9.48 17.45,12 13.09,14.52Z").toPath()
        }
        Canvas(Modifier.size(AndroidControlSize.icon)) {
            scale(size.width / 24f, size.height / 24f, pivot = Offset.Zero) {
                drawPath(left, if (enabled) Color(0xff6d6d6d) else colors.secondary)
                drawPath(right, if (enabled) Color(0xfff1007e) else colors.secondary)
            }
        }
    }
}

@Composable
internal fun AttachmentExportDialog(name: String, size: String?, cancel: () -> Unit, export: (Boolean) -> Unit) {
    androidx.compose.material3.AlertDialog(onDismissRequest = cancel, title = { Text(name) },
        text = { if (size != null) Text(size) },
        confirmButton = { androidx.compose.material3.TextButton(onClick = { export(false) }) { Text("保存") } },
        dismissButton = { androidx.compose.material3.TextButton(onClick = { export(true) }) { Text("共有") } })
}
