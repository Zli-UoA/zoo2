package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.foundation.focusable
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
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.Measurable
import androidx.compose.ui.layout.Placeable
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class InputState(val text: String = "", val expanded: Boolean = false, val canSend: Boolean = true, val hasAttachments: Boolean = false, val mentions: List<zoo.CompletionOption> = emptyList(),
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
    onResizeChanged: (Boolean) -> Unit = {},
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
    var dismissed by remember(state.text, state.selection) { mutableStateOf(false) }
    var selectedOption by remember(state.text, state.selection) { mutableIntStateOf(0) }
    val options = state.mentions.takeIf { focused && !dismissed }.orEmpty()
    val activeOption = selectedOption.coerceAtMost(options.lastIndex.coerceAtLeast(0))
    var suggestionsVisible by remember { mutableStateOf(false) }
    val full = state.expanded
    var dragDistance by remember(full) { mutableStateOf(0.dp) }
    var compactHeight by remember { mutableStateOf(0.dp) }
    val showTools = full || focused || state.text.isNotEmpty()
    val density = LocalDensity.current
    val expand by rememberUpdatedState { onTool(InputTool.Expand) }
    val resizeChanged by rememberUpdatedState(onResizeChanged)
    BoxWithConstraints(modifier.fillMaxWidth(), contentAlignment = Alignment.BottomCenter) {
        val minimum = maxOf(compactHeight, AndroidControlSize.target * 2 + AndroidControlSize.inputHandleHeight)
            .coerceAtMost(maxHeight)
        val maxTravel = if (full) maxHeight - minimum else (maxHeight - compactHeight).coerceAtLeast(0.dp)
        val drag = if (full) dragDistance.coerceIn(-maxTravel, 0.dp) else dragDistance.coerceIn(0.dp, maxTravel)
        val height = when {
            full -> (maxHeight + drag).coerceIn(minimum, maxHeight)
            drag > 0.dp -> (compactHeight + drag).coerceAtMost(maxHeight)
            else -> null
        }
        val adjustable = height != null
        val dragThreshold by rememberUpdatedState(
            if (maxTravel > 0.dp) minOf(AndroidControlSize.inputExpandThreshold, maxTravel)
            else AndroidControlSize.inputExpandThreshold,
        )
        val field: @Composable (Modifier) -> Unit = { placement ->
            BasicTextField(
                value = value,
                onValueChange = {
                    fieldValue = it
                    if (it.text != state.text) textChanged(it.text)
                    if (state.selection != it.selection) onSelection(it.selection)
                },
                modifier = placement.focusRequester(focusRequester).onFocusChanged { focused = it.isFocused }
                    .onPreviewKeyEvent { event ->
                        if (event.type != KeyEventType.KeyDown || value.composition != null || options.isEmpty() || !suggestionsVisible ||
                            event.isCtrlPressed || event.isMetaPressed || event.isShiftPressed || event.isAltPressed
                        ) {
                            false
                        } else when (event.key) {
                            Key.DirectionUp, Key.DirectionDown -> {
                                val step = if (event.key == Key.DirectionDown) 1 else -1
                                selectedOption = (activeOption + step + options.size) % options.size
                                true
                            }
                            Key.Enter, Key.Tab -> {
                                onMention(options[activeOption].name)
                                true
                            }
                            Key.Escape -> {
                                dismissed = true
                                true
                            }
                            else -> false
                        }
                    }
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
        Layout(
            modifier = Modifier.fillMaxWidth()
            .then(if (height != null) Modifier.height(height) else Modifier)
            .onSizeChanged {
                if (!full && drag == 0.dp) compactHeight = with(density) { it.height.toDp() }
            }
            .background(colors.inputSurface).padding(bottom = AndroidControlSize.inputEdgeInset),
            contents = listOf(
                {
                    if (state.notices.isNotEmpty()) {
                        Column(Modifier.fillMaxWidth().testTag("encryption-recipients").verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp)
                            .semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
                            verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            state.notices.forEach { Text(it, color = colors.inputHint, fontSize = 12.sp) }
                        }
                    }
                },
                {
                    if (options.isNotEmpty()) {
                        val shape = RoundedCornerShape(8.dp)
                        Column(Modifier.fillMaxWidth().padding(horizontal = 8.dp)
                            .clip(shape).background(colors.quietSurface)
                            .border(1.dp, colors.inputBoundary, shape).padding(4.dp)
                            .semantics { if (!suggestionsVisible) hideFromAccessibility() }
                            .verticalScroll(rememberScrollState())) {
                            options.forEachIndexed { index, option ->
                                val bringIntoView = remember { BringIntoViewRequester() }
                                LaunchedEffect(index == activeOption) {
                                    if (index == activeOption) bringIntoView.bringIntoView()
                                }
                                Row(
                                    Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
                                        .bringIntoViewRequester(bringIntoView)
                                        .clip(RoundedCornerShape(4.dp))
                                        .background(if (index == activeOption) colors.suggestionSelected else Color.Transparent)
                                        .semantics { selected = index == activeOption }
                                        .clickable(enabled = suggestionsVisible && value.composition == null) { onMention(option.name) }
                                        .padding(horizontal = 8.dp, vertical = 4.dp),
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Text("@${option.name}", color = colors.foreground, fontSize = 14.sp,
                                        fontFamily = FontFamily.Monospace)
                                    Text(option.label, color = colors.secondary, fontSize = 14.sp, maxLines = 1,
                                        overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                                }
                            }
                        }
                    }
                },
                {
                    if (canResize && (!hideIdleHandle || showTools)) Box(
                        Modifier.fillMaxWidth().height(AndroidControlSize.inputHandleHeight)
                            .pointerInput(full) {
                                fun resetDrag() {
                                    dragDistance = 0.dp
                                    resizeChanged(false)
                                }
                                try {
                                    detectVerticalDragGestures(
                                        onDragStart = { resizeChanged(true) },
                                        onDragCancel = ::resetDrag,
                                        onDragEnd = {
                                            val distance = if (full) -dragDistance else dragDistance
                                            if (distance >= dragThreshold) expand()
                                            resetDrag()
                                        },
                                    ) { _, delta -> dragDistance -= delta.toDp() }
                                } finally {
                                    resetDrag()
                                }
                            }.onKeyEvent { event ->
                                val activatesHandle = when (event.key) {
                                    Key.Enter, Key.NumPadEnter, Key.DirectionCenter, Key.Spacebar -> true
                                    else -> false
                                }
                                if (event.type == KeyEventType.KeyUp && activatesHandle) {
                                    expand()
                                    true
                                } else false
                            }.focusable().semantics {
                                val action = if (full) "入力欄を縮める" else "入力欄を広げる"
                                contentDescription = action
                                customActions = listOf(CustomAccessibilityAction(action) { expand(); true })
                            },
                        contentAlignment = Alignment.Center,
                    ) {
                        Box(Modifier.size(28.dp, 3.dp).background(colors.inputHint, CircleShape))
                    }
                },
                {
                    Row(
                        Modifier.fillMaxWidth()
                            .padding(horizontal = if (showTools) 0.dp else AndroidControlSize.inputEdgeInset),
                        verticalAlignment = if (adjustable) Alignment.Top else Alignment.CenterVertically,
                    ) {
                        if (!showTools && !state.live) UiIconButton(UiIcon.Attach, "添付", colors.inputText, { onTool(InputTool.Attach) })
                        field(Modifier.weight(1f).then(if (adjustable) Modifier.fillMaxHeight() else Modifier))
                        if (!showTools) SendAction(state, colors, onTool)
                    }
                },
                {
                    Column(Modifier.fillMaxWidth()) {
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
                },
            ),
        ) { slots, constraints ->
            val (noticesSlot, suggestionsSlot, handleSlot, editorSlot, controlsSlot) = slots
            val available = constraints.copy(minWidth = 0, minHeight = 0)
            fun List<Measurable>.measureWithin(height: Int, fill: Boolean = false) =
                singleOrNull()?.measure(available.copy(minHeight = if (fill) height else 0, maxHeight = height))
            fun spaceAfter(height: Int, used: Int) =
                if (height == Constraints.Infinity) height else (height - used).coerceAtLeast(0)

            val controls = controlsSlot.measureWithin(available.maxHeight)
            var remaining = spaceAfter(available.maxHeight, controls?.height ?: 0)
            val handle = handleSlot.measureWithin(remaining)
            remaining = spaceAfter(remaining, handle?.height ?: 0)
            // Keep room for the editor even when recipient warnings take multiple lines.
            val editorMinimum = (23.sp.roundToPx() + 16.dp.roundToPx()).coerceAtLeast(AndroidControlSize.target.roundToPx())
            val notices = noticesSlot.measureWithin(
                if (remaining == Constraints.Infinity) 192.dp.roundToPx() else spaceAfter(remaining, editorMinimum)
            )
            remaining = spaceAfter(remaining, notices?.height ?: 0)
            val minimumSuggestionHeight = AndroidControlSize.target.roundToPx() + 8.dp.roundToPx()
            fun measureSuggestions(height: Int) = if (height >= minimumSuggestionHeight)
                suggestionsSlot.measureWithin(minOf(height, 192.dp.roundToPx())) else null

            val suggestions: Placeable?
            val editor: Placeable?
            if (adjustable) {
                suggestions = measureSuggestions(spaceAfter(remaining, editorMinimum))
                editor = editorSlot.measureWithin(spaceAfter(remaining, suggestions?.height ?: 0), fill = true)
            } else {
                editor = editorSlot.measureWithin(remaining)
                suggestions = measureSuggestions(spaceAfter(remaining, editor?.height ?: 0))
            }
            suggestionsVisible = suggestions != null

            val placed = listOfNotNull(notices, suggestions, handle, editor)
            val usedHeight = placed.sumOf { it.height } + (controls?.height ?: 0)
            val layoutHeight = usedHeight.coerceIn(constraints.minHeight, constraints.maxHeight)
            layout(constraints.maxWidth, layoutHeight) {
                var y = 0
                placed.forEach {
                    it.placeRelative(0, y)
                    y += it.height
                }
                controls?.placeRelative(0, layoutHeight - controls.height)
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
