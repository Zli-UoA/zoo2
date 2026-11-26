package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.TextButton
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class InputState(val text: String = "", val expanded: Boolean = false, val canSend: Boolean = true, val hasAttachments: Boolean = false, val mentions: List<String> = emptyList(),
    val selection: TextRange? = null, val files: List<Pair<String, String>> = emptyList(), val live: Boolean = false, val focusRequest: Int = 0,
    /** 暗号化チャンネルの送信画面の表示（増えた配布先と、鍵を配れない相手。#565）。何もなければ空。 */
    val notices: List<String> = emptyList(),
    val lastSend: String? = null,
)
internal enum class InputTool { Attach, Format, Emoji, Send, Expand }

@Composable
internal fun ConversationInput(
    state: InputState,
    colors: AndroidColors,
    textChanged: (String) -> Unit,
    onTool: (InputTool) -> Unit,
    modifier: Modifier = Modifier,
    onMention: (String) -> Unit = {},
    onSelection: (TextRange) -> Unit = {},
    onRemoveFile: (String) -> Unit = {},
    hideIdleHandle: Boolean = false,
    canResize: Boolean = true,
) {
    var fieldValue by remember { mutableStateOf(TextFieldValue(state.text, state.selection ?: TextRange(state.text.length))) }
    val value = if (fieldValue.text == state.text && (state.selection == null || fieldValue.selection == state.selection)) {
        fieldValue
    } else TextFieldValue(state.text, state.selection ?: TextRange(state.text.length))
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(state.focusRequest) {
        if (state.focusRequest != 0) focusRequester.requestFocus()
    }
    var focused by remember { mutableStateOf(false) }
    val full = state.expanded
    var drag by remember(full) { mutableStateOf(0.dp) }
    var compactHeight by remember { mutableStateOf(0.dp) }
    val showTools = full || focused || state.text.isNotEmpty()
    val density = LocalDensity.current
    val expand by rememberUpdatedState { onTool(InputTool.Expand) }
    BoxWithConstraints(modifier.fillMaxWidth(), contentAlignment = Alignment.BottomCenter) {
        val minimum = maxOf(compactHeight, AndroidControlSize.target * 2 + AndroidControlSize.inputHandleHeight)
            .coerceAtMost(maxHeight)
        val maxTravel = if (full) maxHeight - minimum else (maxHeight - compactHeight).coerceAtLeast(0.dp)
        val height = when {
            full -> (maxHeight + drag).coerceIn(minimum, maxHeight)
            drag > 0.dp -> (compactHeight + drag).coerceAtMost(maxHeight)
            else -> null
        }
        val adjustable = height != null
        val dragLimit by rememberUpdatedState(maxTravel)
        Column(Modifier.fillMaxWidth()
            .then(if (height != null) Modifier.height(height) else Modifier)
            .onSizeChanged {
                if (!full && drag == 0.dp) compactHeight = with(density) { it.height.toDp() }
            }
            .background(colors.inputSurface).padding(bottom = AndroidControlSize.inputEdgeInset)) {
            if (state.notices.isNotEmpty()) {
                Column(Modifier.fillMaxWidth().testTag("encryption-recipients").padding(horizontal = 16.dp, vertical = 8.dp)
                    .semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
                    verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    state.notices.forEach { Text(it, color = colors.inputHint, fontSize = 12.sp) }
                }
            }
            if (state.mentions.isNotEmpty()) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp)
                    .horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    state.mentions.forEach { name ->
                        TextAction("@$name", colors, style = TextActionStyle.Plain) { onMention(name) }
                    }
                }
            }
            if (canResize && (!hideIdleHandle || showTools)) Box(
                Modifier.fillMaxWidth().height(AndroidControlSize.inputHandleHeight)
                    .pointerInput(full) {
                        detectVerticalDragGestures(
                            onDragCancel = { drag = 0.dp },
                            onDragEnd = {
                                val distance = if (full) -drag else drag
                                if (distance > 0.dp && distance >= minOf(AndroidControlSize.inputExpandThreshold, dragLimit)) {
                                    expand()
                                }
                                drag = 0.dp
                            },
                        ) { _, delta ->
                            val heightChange = drag - delta.toDp()
                            drag = if (full) heightChange.coerceIn(-dragLimit, 0.dp)
                                else heightChange.coerceIn(0.dp, dragLimit)
                        }
                    }.clickable(role = Role.Button, onClick = expand)
                    .semantics { contentDescription = if (full) "入力欄を縮める" else "入力欄を広げる" },
                contentAlignment = Alignment.Center,
            ) {
                Box(Modifier.size(28.dp, 3.dp).background(colors.inputHint, CircleShape))
            }
            val field: @Composable (Modifier) -> Unit = { placement ->
                BasicTextField(
                    value = value,
                    onValueChange = {
                        fieldValue = it
                        if (it.text != state.text) textChanged(it.text)
                        if (state.selection != it.selection) onSelection(it.selection)
                    },
                    modifier = placement.focusRequester(focusRequester).onFocusChanged { focused = it.isFocused }
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                    textStyle = LocalTextStyle.current.copy(color = colors.inputText, fontSize = 16.sp, lineHeight = 23.sp),
                    cursorBrush = SolidColor(colors.inputText),
                    minLines = 1,
                    maxLines = if (adjustable) Int.MAX_VALUE else if (showTools) 6 else 1,
                    decorationBox = { inner ->
                        Box {
                            if (state.text.isEmpty()) Text(if (state.live) "入力内容を共有" else "メッセージを送信", color = colors.inputHint, fontSize = 16.sp)
                            inner()
                        }
                    },
                )
            }
            Row(
                Modifier.fillMaxWidth()
                    .padding(horizontal = if (showTools) 0.dp else AndroidControlSize.inputEdgeInset)
                    .then(if (adjustable) Modifier.weight(1f) else Modifier),
                verticalAlignment = if (adjustable) Alignment.Top else Alignment.CenterVertically,
            ) {
                if (!showTools && !state.live) UiIconButton(UiIcon.Attach, "添付", colors.inputText, { onTool(InputTool.Attach) })
                field(Modifier.weight(1f).then(if (adjustable) Modifier.fillMaxHeight() else Modifier))
                if (!showTools) SendAction(state, colors, onTool)
            }
            if (state.files.isNotEmpty()) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp)
                    .horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    state.files.forEach { (id, name) ->
                        TextAction(name, colors, icon = UiIcon.Close, style = TextActionStyle.Neutral) { onRemoveFile(id) }
                    }
                }
            }
            if (showTools) {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = AndroidControlSize.inputEdgeInset),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (!state.live) {
                        UiIconButton(UiIcon.Attach, "添付", colors.inputText, { onTool(InputTool.Attach) })
                        UiIconButton(UiIcon.Format, "書式", colors.inputText, { onTool(InputTool.Format) })
                        UiIconButton(UiIcon.Emoji, "絵文字", colors.inputText, { onTool(InputTool.Emoji) })
                    }
                    Spacer(Modifier.weight(1f))
                    SendAction(state, colors, onTool)
                }
            }
        }
    }
}

@Composable
private fun SendAction(state: InputState, colors: AndroidColors, onTool: (InputTool) -> Unit) {
    val enabled = (state.text.isNotBlank() || state.hasAttachments) && state.canSend
    UiIconButton(
        if (state.live) UiIcon.Delete else UiIcon.Send,
        if (state.live) "入力を消去" else "送信", if (enabled) Color.Black else colors.inputHint,
        action = { onTool(InputTool.Send) },
        enabled = enabled,
        fill = if (enabled) colors.accent else Color.Transparent,
    )
}


@Composable
internal fun FormattingDialog(onDismiss: () -> Unit, onSelect: (String) -> Unit) {
    val options = listOf("B" to "**太字**", "I" to "*斜体*", "‹/›" to "`コード`", "❝" to "\n> 引用")
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("文字装飾") },
        text = {
            Row {
                options.forEach { (label, value) ->
                    TextButton(onClick = { onSelect(value) }) { Text(label) }
                }
            }
        },
        confirmButton = {},
    )
}
