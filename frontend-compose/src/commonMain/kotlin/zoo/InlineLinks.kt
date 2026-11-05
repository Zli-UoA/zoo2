package zoo

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.GenericShape
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.IntOffset
import kotlin.math.ceil
import kotlin.math.floor

internal val LocalInlineLinksEnabled = staticCompositionLocalOf { true }

/**
 * Keep native paragraph layout/selection while exposing each link as a named, keyboardable link.
 */
@Composable
internal fun InlineLinks(
    text: AnnotatedString,
    content: @Composable (AnnotatedString, (TextLayoutResult) -> Unit) -> Unit,
) {
    val links = remember(text) { text.getLinkAnnotations(0, text.length) }
    if (links.isEmpty()) {
        content(text) {}
        return
    }
    val display =
        remember(text) {
            buildAnnotatedString {
                append(text.text)
                text.spanStyles.forEach { addStyle(it.item, it.start, it.end) }
                text.paragraphStyles.forEach { addStyle(it.item, it.start, it.end) }
                text.getStringAnnotations(0, text.length).forEach {
                    addStringAnnotation(it.tag, it.item, it.start, it.end)
                }
                links.forEach {
                    val inherited =
                        text.spanStyles
                            .filter { span -> span.start <= it.start && span.end >= it.end }
                            .mapNotNull { span -> span.item.textDecoration }
                    addStyle(
                        SpanStyle(
                            textDecoration =
                                TextDecoration.combine(inherited + TextDecoration.Underline)
                        ),
                        it.start,
                        it.end,
                    )
                }
            }
        }
    var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
    val density = LocalDensity.current
    val enabled = LocalInlineLinksEnabled.current
    Box {
        content(display) { layout = it }
        layout
            ?.takeIf { enabled }
            ?.let { result ->
                links.forEach { range ->
                    val url = (range.item as? LinkAnnotation.Url)?.url ?: return@forEach
                    val path = result.getPathForRange(range.start, range.end)
                    val bounds = path.getBounds()
                    if (bounds.width > 0f && bounds.height > 0f) {
                        val localPath =
                            Path().apply { addPath(path, Offset(-bounds.left, -bounds.top)) }
                        val shape = GenericShape { _, _ -> addPath(localPath) }
                        val x = floor(bounds.left).toInt()
                        val y = floor(bounds.top).toInt()
                        NativeLink(
                            url,
                            Modifier.offset { IntOffset(x, y) }
                                .graphicsLayer {
                                    translationX = bounds.left - x
                                    translationY = bounds.top - y
                                }
                                .size(
                                    with(density) { ceil(bounds.width).toDp() },
                                    with(density) { ceil(bounds.height).toDp() },
                                )
                                .clip(shape)
                                .semantics {
                                    contentDescription = text.text.substring(range.start, range.end)
                                },
                            attributes =
                                if (url.startsWith('/')) emptyMap()
                                else mapOf("target" to "_blank", "rel" to "noopener noreferrer"),
                        ) {}
                    }
                }
            }
    }
}
