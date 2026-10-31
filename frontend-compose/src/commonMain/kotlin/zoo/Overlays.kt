package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.focus.*
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.key.*
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*

/** Overlays share the application's scene and accessibility owner on every target. */
@Stable
class OverlaySlot {
    var title by mutableStateOf("")
    var width by mutableStateOf(512.dp)
    var anchor by mutableStateOf<Rect?>(null)
    var close: () -> Unit = {}
    var body by mutableStateOf<@Composable ColumnScope.() -> Unit>({})
}

@Composable
fun OverlayRegistration(
    title: String,
    close: () -> Unit,
    width: Dp,
    anchor: Rect? = null,
    body: @Composable ColumnScope.() -> Unit,
) {
    val app = LocalApp.current
    val slot = remember { OverlaySlot() }
    SideEffect {
        slot.title = title
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
    LaunchedEffect(slot) { focus.requestFocus() }
    BoxWithConstraints(
        Modifier.fillMaxSize()
            .background(
                if (slot.anchor == null) androidx.compose.ui.graphics.Color.Black.copy(alpha = .5f)
                else androidx.compose.ui.graphics.Color.Transparent
            )
            .pointerInput(slot) { detectTapGestures { slot.close() } }
            .onPreviewKeyEvent {
                if (it.type == KeyEventType.KeyDown && it.key == Key.Escape) {
                    app.escape()
                    true
                } else false
            }
            .focusRequester(focus)
            .focusable()
    ) {
        val anchor = slot.anchor
        val screenWidth = maxWidth
        val screenHeight = maxHeight
        val position =
            if (anchor == null) Modifier.align(Alignment.Center)
            else
                with(density) {
                    Modifier.offset(
                        anchor.left
                            .toDp()
                            .coerceIn(8.dp, (screenWidth - slot.width - 8.dp).coerceAtLeast(8.dp)),
                        anchor.bottom
                            .toDp()
                            .coerceIn(
                                8.dp,
                                (screenHeight - measured.height.toDp() - 8.dp).coerceAtLeast(8.dp),
                            ),
                    )
                }
        Surface(
            position
                .widthIn(max = slot.width.coerceAtMost(screenWidth - 32.dp))
                .onSizeChanged { measured = it }
                .pointerInput(slot) { detectTapGestures {} }
                .semantics {
                    paneTitle = slot.title
                    if (slot.anchor == null) dialog()
                    isTraversalGroup = true
                },
            shape = RoundedCornerShape(if (anchor == null) 12.dp else 8.dp),
            color = MaterialTheme.colorScheme.background,
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
            shadowElevation = 8.dp,
        ) {
            Column(
                Modifier.heightIn(max = (screenHeight - 32.dp).coerceAtMost(700.dp))
                    .padding(if (anchor == null) 24.dp else 4.dp),
                verticalArrangement = Arrangement.spacedBy(if (anchor == null) 12.dp else 0.dp),
            ) {
                if (anchor == null)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            slot.title,
                            Modifier.weight(1f),
                            weight = FontWeight.SemiBold,
                            size = 18.sp,
                        )
                        IconButton("x", app.t("common.close"), click = slot.close)
                    }
                slot.body(this)
            }
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
    change: (String) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    var anchor by remember { mutableStateOf(Rect.Zero) }
    Box(Modifier.onGloballyPositioned { anchor = it.boundsInRoot() }) {
        Button("$label: $value", enabled) { open = true }
    }
    if (open)
        OverlayRegistration(label, { open = false }, 256.dp, anchor) {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                ((if (allowEmpty) listOf("") else emptyList()) + options).forEach { option ->
                    Button(option.ifEmpty { "—" }, modifier = Modifier.fillMaxWidth()) {
                        open = false
                        change(option)
                    }
                }
            }
        }
}
