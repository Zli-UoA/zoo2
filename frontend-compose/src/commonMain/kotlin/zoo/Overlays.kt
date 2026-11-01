package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.*
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.key.*
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*

private var overlaySequence = 0

/** Overlays share the application's scene and accessibility owner on every target. */
@Stable
class OverlaySlot {
    val id = "overlay-${++overlaySequence}"
    var role: String? = null
    var nonModal = false
    var tooltip = false
    var placementAbove = false
    var alignEnd = false
    var anchorOffset = 0.dp
    var description by mutableStateOf("")
    var footer by mutableStateOf<(@Composable () -> Unit)?>(null)
    var alert by mutableStateOf(false)
    var title by mutableStateOf("")
    var width by mutableStateOf(512.dp)
    var anchor by mutableStateOf<Rect?>(null)
    var onEscape: (() -> Boolean)? = null
    var close: () -> Unit = {}
    var body by mutableStateOf<@Composable ColumnScope.() -> Unit>({})
}

@Composable
fun OverlayRegistration(
    title: String,
    close: () -> Unit,
    width: Dp,
    anchor: Rect? = null,
    alert: Boolean = false,
    description: String = "",
    footer: (@Composable () -> Unit)? = null,
    role: String? = null,
    nonModal: Boolean = false,
    tooltip: Boolean = false,
    placementAbove: Boolean = false,
    alignEnd: Boolean = false,
    anchorOffset: Dp = 0.dp,
    onEscape: (() -> Boolean)? = null,
    body: @Composable ColumnScope.() -> Unit,
) {
    val app = LocalApp.current
    val slot = remember { OverlaySlot() }
    SideEffect {
        slot.title = title
        slot.alert = alert
        slot.description = description
        slot.footer = footer
        slot.role = role
        slot.nonModal = nonModal
        slot.tooltip = tooltip
        slot.placementAbove = placementAbove
        slot.alignEnd = alignEnd
        slot.anchorOffset = anchorOffset
        slot.onEscape = onEscape
        slot.close = close
        slot.width = width
        slot.anchor = anchor
        slot.body = body
    }
    DisposableEffect(app, slot) {
        app.overlays.add(slot)
        onDispose { app.overlays.remove(slot) }
    }
}

@Composable
fun OverlayHost(app: Session) {
    app.overlays.toList().forEach { slot -> key(slot) { OverlayCard(slot) } }
}

@Composable
private fun OverlayCard(slot: OverlaySlot) {
    val app = LocalApp.current
    val focus = remember { FocusRequester() }
    val density = LocalDensity.current
    var measured by remember { mutableStateOf(IntSize.Zero) }
    var bounds by remember { mutableStateOf(Rect.Zero) }
    // Request the popup surface, never its fullscreen backdrop. Keep this effect before
    // composing the body so a body's initial field-focus effect can take focus afterwards.
    LaunchedEffect(slot) { if (!slot.nonModal && !slot.tooltip) focus.requestFocus() }
    SideEffect {
        if (slot.nonModal && !slot.tooltip && bounds.width > 0)
            with(density) {
                app.platform.outsideClick(
                    slot.id,
                    obj(
                            "left" to bounds.left.toDp().value.toString(),
                            "top" to bounds.top.toDp().value.toString(),
                            "right" to bounds.right.toDp().value.toString(),
                            "bottom" to bounds.bottom.toDp().value.toString(),
                        )
                        .toString(),
                    slot.close,
                )
            }
    }
    DisposableEffect(slot) { onDispose { app.platform.forgetOutsideClick(slot.id) } }
    val backdrop =
        if (slot.anchor == null && !slot.nonModal)
            androidx.compose.ui.graphics.Color.Black.copy(alpha = .1f)
        else androidx.compose.ui.graphics.Color.Transparent
    val input =
        if (slot.nonModal || slot.tooltip) Modifier
        else Modifier.pointerInput(slot) { detectTapGestures { if (!slot.alert) slot.close() } }

    BoxWithConstraints(Modifier.fillMaxSize().background(backdrop).then(input)) {
        val screenWidth = maxWidth
        val screenHeight = maxHeight
        val anchor = slot.anchor
        // Base UI prefers the top side, then flips when the lower side has more room.
        // Keep the arrow aimed at the trigger when viewport collision shifts the popup.
        val popupWidth = with(density) { measured.width.toDp() }
        val popupHeight = with(density) { measured.height.toDp() }
        val tooltipAbove =
            anchor?.let {
                with(density) {
                    val roomAbove = it.top.toDp() - 8.dp
                    val roomBelow = screenHeight - it.bottom.toDp() - 8.dp
                    roomAbove >= popupHeight + 4.dp || roomAbove >= roomBelow
                }
            } ?: true
        val tooltipLeft =
            anchor?.let {
                with(density) {
                    (it.center.x.toDp() - popupWidth / 2).coerceIn(
                        8.dp,
                        (screenWidth - popupWidth - 8.dp).coerceAtLeast(8.dp),
                    )
                }
            } ?: 8.dp
        val tooltipTop =
            anchor?.let {
                with(density) {
                    (if (tooltipAbove) it.top.toDp() - popupHeight - 4.dp
                        else it.bottom.toDp() + 4.dp)
                        .coerceIn(8.dp, (screenHeight - popupHeight - 8.dp).coerceAtLeast(8.dp))
                }
            } ?: 8.dp
        val position =
            if (anchor == null) Modifier.align(Alignment.Center)
            else if (slot.tooltip) Modifier.offset(tooltipLeft, tooltipTop)
            else
                with(density) {
                    val left =
                        if (slot.alignEnd) anchor.right.toDp() - measured.width.toDp()
                        else anchor.left.toDp()
                    val top =
                        if (slot.placementAbove) anchor.top.toDp() - measured.height.toDp() - 4.dp
                        else anchor.bottom.toDp() + slot.anchorOffset
                    Modifier.offset(
                        left.coerceIn(
                            8.dp,
                            (screenWidth - measured.width.toDp() - 8.dp).coerceAtLeast(8.dp),
                        ),
                        top.coerceIn(
                            8.dp,
                            (screenHeight - measured.height.toDp() - 8.dp).coerceAtLeast(8.dp),
                        ),
                    )
                }
        val width =
            if (slot.alert && screenWidth < 640.dp) slot.width.coerceAtMost(320.dp) else slot.width
        val size =
            if (slot.tooltip) Modifier.widthIn(max = width.coerceAtMost(screenWidth - 16.dp))
            else Modifier.width(width.coerceAtMost(screenWidth - 32.dp))
        val role =
            slot.role ?: if (slot.alert) "alertdialog" else if (anchor == null) "dialog" else "menu"
        val menuRingColor = MaterialTheme.colorScheme.onSurface.copy(alpha = .1f)
        val menuRing =
            if (!slot.tooltip)
                Modifier.drawBehind {
                    val stroke = 1.dp.toPx()
                    drawRoundRect(
                        menuRingColor,
                        Offset(-stroke / 2, -stroke / 2),
                        Size(this.size.width + stroke, this.size.height + stroke),
                        CornerRadius((if (anchor == null) 14.dp else 10.dp).toPx() + stroke / 2),
                        style = Stroke(stroke),
                    )
                }
            else Modifier
        val cardShape =
            RoundedCornerShape(if (slot.tooltip) 8.dp else if (anchor == null) 14.dp else 10.dp)
        Surface(
            position
                .then(menuRing)
                .then(
                    if (anchor != null && !slot.tooltip) Modifier.cssShadowMd(cardShape)
                    else Modifier
                )
                .then(size)
                .onSizeChanged { measured = it }
                .onGloballyPositioned { bounds = it.boundsInRoot() }
                .then(
                    if (slot.nonModal || slot.tooltip) Modifier
                    else Modifier.focusRequester(focus).focusable()
                )
                .then(
                    webControlModifier(
                        role,
                        attributes =
                            if (slot.tooltip) emptyMap()
                            else
                                buildMap {
                                    put("aria-label", slot.title)
                                    if (anchor == null && !slot.nonModal) put("aria-modal", "true")
                                },
                    )
                )
                .then(
                    if (slot.tooltip) Modifier
                    else Modifier.pointerInput(slot) { detectTapGestures {} }
                )
                .semantics {
                    paneTitle = slot.title
                    if (anchor == null && !slot.tooltip) dialog()
                    isTraversalGroup = true
                },
            shape = cardShape,
            color =
                if (slot.tooltip) MaterialTheme.colorScheme.onBackground
                else MaterialTheme.colorScheme.surface,
            border = null,
            shadowElevation = 0.dp,
        ) {
            if (slot.tooltip)
                Column(Modifier.padding(horizontal = 12.dp, vertical = 6.dp)) { slot.body(this) }
            else
                Column(Modifier.heightIn(max = screenHeight - 32.dp)) {
                    Column(
                        Modifier.padding(if (anchor == null) 16.dp else 4.dp),
                        verticalArrangement =
                            Arrangement.spacedBy(if (anchor == null) 16.dp else 0.dp),
                    ) {
                        if (anchor == null)
                            Column(
                                verticalArrangement =
                                    Arrangement.spacedBy(if (slot.alert) 6.dp else 8.dp)
                            ) {
                                var titleLayout by
                                    remember(slot.title) {
                                        mutableStateOf<androidx.compose.ui.text.TextLayoutResult?>(
                                            null
                                        )
                                    }
                                Box(Modifier.fillMaxWidth()) {
                                    Text(
                                        slot.title,
                                        Modifier.padding(end = if (slot.alert) 0.dp else 24.dp)
                                            .then(
                                                if (slot.alert) Modifier
                                                else
                                                    Modifier.cssTightTextLineBox(16.dp) {
                                                        titleLayout
                                                    }
                                            )
                                            .then(
                                                webControlModifier(
                                                    "heading",
                                                    attributes = mapOf("aria-level" to "2"),
                                                )
                                            ),
                                        weight = FontWeight.Medium,
                                        size = 16.sp,
                                        lineHeight = if (slot.alert) 24.sp else 16.sp,
                                        onTextLayout = { titleLayout = it },
                                    )
                                    if (!slot.alert)
                                        Box(Modifier.matchParentSize()) {
                                            IconButton(
                                                "x",
                                                app.t("common.close"),
                                                size = 28.dp,
                                                color = MaterialTheme.colorScheme.onSurface,
                                                modifier =
                                                    Modifier.align(Alignment.TopEnd)
                                                        .offset(x = 8.dp, y = (-8).dp),
                                                click = slot.close,
                                            )
                                        }
                                }
                                if (slot.description.isNotEmpty())
                                    Text(slot.description, muted = true)
                            }
                        slot.body(this)
                    }
                    slot.footer?.invoke()
                }
        }
        if (slot.tooltip && anchor != null && measured.width > 0) {
            val arrowCenter =
                with(density) { anchor.center.x.toDp() - tooltipLeft }
                    .coerceIn(7.dp, (popupWidth - 7.dp).coerceAtLeast(7.dp))
            // Match the 10px, 45-degree rounded square used by TooltipPrimitive.Arrow.
            Box(
                Modifier.offset(
                        tooltipLeft + arrowCenter - 5.dp,
                        tooltipTop + if (tooltipAbove) popupHeight - 7.dp else (-3).dp,
                    )
                    .size(10.dp)
                    .graphicsLayer { rotationZ = 45f }
                    .background(MaterialTheme.colorScheme.onBackground, RoundedCornerShape(2.dp))
            )
        }
    }
}

@Composable
fun SelectField(
    label: String,
    value: String,
    options: List<String>,
    enabled: Boolean = true,
    allowEmpty: Boolean = true,
    busy: Boolean = false,
    change: (String) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    var anchor by remember { mutableStateOf(Rect.Zero) }
    val app = LocalApp.current
    val choices = (if (allowEmpty) listOf("") else emptyList()) + options
    Box(
        Modifier.graphicsLayer { alpha = if (enabled) 1f else .6f }
            .onGloballyPositioned { anchor = it.boundsInRoot() }
            .then(webControlModifier("combobox", enabled, expanded = open, busy = busy))
            .width(160.dp)
            .height(32.dp)
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(10.dp))
            .background(MaterialTheme.colorScheme.background, RoundedCornerShape(10.dp))
            .onPreviewKeyEvent {
                if (!enabled || it.type != KeyEventType.KeyDown) false
                else {
                    val current = choices.indexOf(value).coerceAtLeast(0)
                    val next =
                        when (it.key) {
                            Key.DirectionDown -> (current + 1).coerceAtMost(choices.lastIndex)
                            Key.DirectionUp -> (current - 1).coerceAtLeast(0)
                            Key.MoveHome -> 0
                            Key.MoveEnd -> choices.lastIndex
                            else -> -1
                        }
                    if (next >= 0) {
                        change(choices[next])
                        true
                    } else false
                }
            }
            .clickable(enabled) { open = !open }
            .semantics {
                contentDescription = label
                if (!enabled) disabled()
            }
            .padding(start = 13.dp, end = 3.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        Text(
            value,
            Modifier.padding(end = 16.dp),
            maxLines = 1,
            color = MaterialTheme.colorScheme.onBackground,
        )
        Icon(
            "chevron-down",
            modifier = Modifier.align(Alignment.CenterEnd).size(16.dp),
            color = MaterialTheme.colorScheme.onBackground,
        )
    }
    if (open)
        OverlayRegistration(label, { open = false }, 160.dp, anchor) {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                choices.forEach { option ->
                    Box(
                        Modifier.fillMaxWidth()
                            .then(webControlModifier("option", selected = option == value))
                            .clickable {
                                open = false
                                change(option)
                            }
                            .padding(horizontal = 6.dp, vertical = 4.dp)
                    ) {
                        Text(option.ifEmpty { "—" })
                    }
                }
            }
        }
}
