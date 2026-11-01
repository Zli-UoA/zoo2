package zoo

import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.*
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.semantics

internal expect fun nativeSelectedText(): String

internal expect fun nativeAbsoluteUrl(url: String): String

private fun navigationAllowed(url: String): Boolean =
    url.none { it.code < 0x20 } &&
        (url.startsWith("/") && !url.startsWith("//") || publicUrl(url) != null)

@Composable
fun Modifier.nativeNavigation(
    url: String,
    role: String? = "link",
    busy: Boolean? = null,
    attributes: Map<String, String> = emptyMap(),
): Modifier {
    val app = LocalApp.current
    if (!navigationAllowed(url)) return this
    fun navigate(newTab: Boolean): Boolean {
        if (nativeSelectedText().isNotEmpty()) return false
        if (!newTab && url.startsWith("/")) app.navigate(url)
        else app.platform.action("open:${nativeAbsoluteUrl(url)}")
        return true
    }
    val keyboard =
        if (role == null) Modifier
        else
            Modifier.pointerHoverIcon(PointerIcon.Hand)
                .then(
                    webControlModifier(
                        role,
                        attributes = attributes + mapOf("href" to url),
                        busy = busy,
                    )
                )
                .semantics { onClick { navigate(false) } }
                .onKeyEvent {
                    if (it.type == KeyEventType.KeyUp && it.key == Key.Enter)
                        navigate(it.isCtrlPressed || it.isMetaPressed)
                    else false
                }
                .focusable()
    return then(keyboard)
        .then(if (role == "link") Modifier.preventMessageHold() else Modifier)
        .pointerInput(url, app) {
            awaitEachGesture {
                val down =
                    awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                val middle =
                    down.type == PointerType.Mouse && currentEvent.buttons.isTertiaryPressed
                if (
                    down.type == PointerType.Mouse &&
                        (!currentEvent.buttons.isPrimaryPressed && !(middle && role == "link"))
                )
                    return@awaitEachGesture
                var dragged = false
                while (true) {
                    // Main dispatch runs from descendants toward ancestors. A nested link
                    // therefore consumes its release before the containing notification row.
                    val event = awaitPointerEvent(PointerEventPass.Main)
                    val pointer = event.changes.find { it.id == down.id } ?: break
                    if (
                        (pointer.position - down.position).getDistance() >
                            viewConfiguration.touchSlop
                    )
                        dragged = true
                    if (!pointer.pressed) {
                        if (
                            !dragged &&
                                !pointer.isConsumed &&
                                navigate(
                                    middle ||
                                        event.keyboardModifiers.isCtrlPressed ||
                                        event.keyboardModifiers.isMetaPressed
                                )
                        )
                            pointer.consume()
                        break
                    }
                }
            }
        }
}

@Composable
fun NativeLink(
    url: String,
    modifier: Modifier = Modifier,
    busy: Boolean? = null,
    attributes: Map<String, String> = emptyMap(),
    content: @Composable () -> Unit,
) {
    Box(modifier.nativeNavigation(url, busy = busy, attributes = attributes)) { content() }
}
