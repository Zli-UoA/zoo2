package zoo

import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.layout
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.constrainHeight
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.round

/** CSS leading-none permits a line box shorter than the font's natural metrics. */
internal fun Modifier.cssTightTextLineBox(
    lineHeight: Dp,
    result: () -> TextLayoutResult?,
): Modifier = layout { measurable, constraints ->
    val child = measurable.measure(constraints.copy(minHeight = 0, maxHeight = Int.MAX_VALUE))
    val text = result()
    val height = lineHeight.toPx()
    val ascent = round(text?.firstBaseline ?: height)
    val descent = round((text?.getLineBottom(0) ?: height) - (text?.firstBaseline ?: height))
    val baseline = round(ascent + (height - ascent - descent) / 2)
    val offset = baseline - (text?.firstBaseline ?: baseline)
    val integerOffset = floor(offset).toInt()
    layout(
        child.width,
        constraints.constrainHeight(ceil(height * (text?.lineCount ?: 1)).toInt()),
    ) {
        child.placeRelativeWithLayer(0, integerOffset) { translationY = offset - integerOffset }
    }
}
