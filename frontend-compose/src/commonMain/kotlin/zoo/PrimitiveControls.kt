package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.interaction.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.input.key.*
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import kotlinx.coroutines.launch

/** Shared sidebar section; the action is outside the disclosure's hit target. */
@Composable
fun CollapsibleSection(
    title: String,
    count: String? = null,
    icon: String = "hash",
    actionLabel: String = "チャンネルを作成",
    defaultOpen: Boolean = true,
    action: (() -> Unit)? = null,
    actions: (@Composable () -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    var open by remember(defaultOpen) { mutableStateOf(defaultOpen) }
    val colors = MaterialTheme.colorScheme
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    val focused by interaction.collectIsFocusedAsState()
    val dark = colors.background == Color.Black
    val labelColor =
        if (hovered) {
            if (dark) Color.White else Color(0xff424242)
        } else colors.onSurfaceVariant
    val hoverBackground = if (dark) Color(0xff2e2e2e) else Color(0xfffffbe8)
    val ring = (if (dark) Color(0xffbdbdbd) else Color(0xff737373)).copy(alpha = .5f)
    Column {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 36.dp).padding(horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(
                Modifier.weight(1f)
                    .heightIn(min = 36.dp)
                    .focusInputModality()
                    .focusVisibleRing(
                        8.dp,
                        ring,
                        focused && FocusInputModality.keyboard,
                        width = 2.dp,
                    )
                    .clip(RoundedCornerShape(8.dp))
                    .background(if (hovered) hoverBackground else Color.Transparent)
                    .hoverable(interaction)
                    .then(webControlModifier("button", expanded = open))
                    .clickable(
                        interactionSource = interaction,
                        indication = null,
                        role = Role.Button,
                    ) {
                        open = !open
                    }
                    .semantics {
                        contentDescription = title
                        stateDescription = if (open) "expanded" else "collapsed"
                    }
                    .padding(horizontal = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Icon(
                    "chevron-right",
                    modifier = Modifier.size(12.dp).rotate(if (open) 90f else 0f),
                    color = labelColor,
                )
                Icon(icon, modifier = Modifier.size(16.dp), color = labelColor)
                Text(
                    title,
                    Modifier.weight(1f),
                    small = true,
                    weight = FontWeight.SemiBold,
                    color = labelColor,
                    maxLines = 1,
                )
                if (count != null)
                    Text(
                        count,
                        Modifier.padding(horizontal = 4.dp),
                        small = true,
                        color = labelColor,
                    )
            }
            actions?.invoke()
            if (action != null)
                IconButton(
                    "plus",
                    actionLabel,
                    size = 28.dp,
                    color = colors.onSurface,
                    click = action,
                )
        }
        if (open) content()
    }
}

/** Scroll viewport with the same 10px track and 6px thumb as the web ScrollArea. */
@Composable
fun ScrollArea(
    modifier: Modifier = Modifier,
    contentPadding: Dp = 0.dp,
    content: @Composable ColumnScope.() -> Unit,
) {
    val state = rememberScrollState()
    val scope = rememberCoroutineScope()
    var viewport by remember { mutableStateOf(0) }
    Box(modifier) {
        Column(
            Modifier.fillMaxSize()
                .padding(contentPadding)
                .onSizeChanged { viewport = it.height }
                .verticalScroll(state),
            content = content,
        )
        if (state.maxValue > 0 && viewport > 0) {
            val fraction = viewport.toFloat() / (viewport + state.maxValue)
            val trackHeight =
                with(androidx.compose.ui.platform.LocalDensity.current) {
                    viewport + contentPadding.toPx() * 2 - 2.dp.toPx()
                }
            val scrollScale = trackHeight / (viewport + state.maxValue)
            Box(
                Modifier.align(Alignment.TopEnd)
                    .fillMaxHeight()
                    .width(10.dp)
                    .padding(start = 3.dp, end = 1.dp, top = 1.dp, bottom = 1.dp)
            ) {
                Box(
                    Modifier.fillMaxWidth()
                        .fillMaxHeight(fraction)
                        .offset(
                            y =
                                with(androidx.compose.ui.platform.LocalDensity.current) {
                                    (state.value * scrollScale).toDp()
                                }
                        )
                        .background(MaterialTheme.colorScheme.outline, RoundedCornerShape(100.dp))
                        .pointerInput(state, scrollScale) {
                            detectVerticalDragGestures { change, delta ->
                                change.consume()
                                scope.launch {
                                    state.scrollTo((state.value + delta / scrollScale).toInt())
                                }
                            }
                        }
                )
            }
        }
    }
}

/** A menu shares the application's overlay owner, preserving Escape and click-away handling. */
@Composable
fun ActionMenu(
    label: String,
    initialOpen: Boolean = false,
    iconOnly: Boolean = false,
    actions: List<MenuAction>,
) {
    var open by remember(initialOpen) { mutableStateOf(initialOpen) }
    var anchor by remember { mutableStateOf(Rect.Zero) }
    Box(Modifier.onGloballyPositioned { anchor = it.boundsInRoot() }) {
        if (iconOnly)
            Button(label, variant = "ghost", size = "icon-sm", icon = "ellipsis", expanded = open) {
                open = !open
            }
        else Button(label, variant = "outline", expanded = open) { open = !open }
    }
    val density = androidx.compose.ui.platform.LocalDensity.current
    val menuAnchor =
        with(density) {
            Rect(
                anchor.center.x - 88.dp.toPx(),
                anchor.top,
                anchor.center.x + 88.dp.toPx(),
                anchor.bottom + 4.dp.toPx(),
            )
        }
    if (open)
        OverlayRegistration(label, { open = false }, 176.dp, menuAnchor, role = "menu") {
            actions.forEachIndexed { index, item ->
                if (index == actions.lastIndex)
                    androidx.compose.material3.HorizontalDivider(Modifier.padding(vertical = 4.dp))
                Row(
                    Modifier.fillMaxWidth()
                        .then(webControlModifier("menuitem", item.enabled))
                        .clickable(enabled = item.enabled) {
                            open = false
                            item.action()
                        }
                        .padding(horizontal = 6.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Icon(
                        item.icon,
                        modifier = Modifier.size(16.dp),
                        color =
                            if (index == actions.lastIndex) MaterialTheme.colorScheme.error
                            else MaterialTheme.colorScheme.onSurface,
                    )
                    Text(
                        item.label,
                        color =
                            if (index == actions.lastIndex) MaterialTheme.colorScheme.error
                            else MaterialTheme.colorScheme.onSurface,
                    )
                }
            }
        }
}

internal val LocalControlActivation = staticCompositionLocalOf<(() -> Unit)?> { null }

@Composable
fun Hint(
    label: String,
    initiallyVisible: Boolean = false,
    content: @Composable (Modifier) -> Unit,
) {
    var hovered by remember { mutableStateOf(false) }
    var focused by remember { mutableStateOf(false) }
    var dismissed by remember { mutableStateOf(false) }
    var anchor by remember { mutableStateOf(Rect.Zero) }
    Box(Modifier.onGloballyPositioned { anchor = it.boundsInRoot() }) {
        CompositionLocalProvider(LocalControlActivation provides { dismissed = true }) {
            content(
                Modifier.onPreviewKeyEvent {
                        if (it.type == KeyEventType.KeyDown) {
                            if (it.key == Key.Enter || it.key == Key.Spacebar) dismissed = true
                            if (
                                it.key == Key.Escape &&
                                    !dismissed &&
                                    (initiallyVisible || hovered || focused)
                            ) {
                                dismissed = true
                                return@onPreviewKeyEvent true
                            }
                        }
                        false
                    }
                    .pointerInput(Unit) {
                        awaitPointerEventScope {
                            while (true) {
                                when (awaitPointerEvent(PointerEventPass.Initial).type) {
                                    PointerEventType.Enter -> {
                                        hovered = true
                                        dismissed = false
                                    }
                                    PointerEventType.Exit -> hovered = false
                                    PointerEventType.Press -> dismissed = true
                                    else -> Unit
                                }
                            }
                        }
                    }
                    .onFocusChanged {
                        focused = it.hasFocus
                        if (!focused) dismissed = false
                    }
            )
        }
    }
    if (!dismissed && (initiallyVisible || hovered || focused))
        OverlayRegistration(
            label,
            { dismissed = true },
            320.dp,
            anchor,
            role = "tooltip",
            nonModal = true,
            tooltip = true,
        ) {
            Text(label, small = true, color = MaterialTheme.colorScheme.background)
        }
}
