package zoo

import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.ParentDataModifier
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.constrainHeight
import androidx.compose.ui.unit.constrainWidth
import androidx.compose.ui.unit.dp
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max

/** CSS layout retains fractional CSS pixel advances between adjacent inline controls. */
data class CssPreferredWidth(val width: Float)

private data class CssPreferredWidthModifier(val width: Float) : ParentDataModifier {
    override fun Density.modifyParentData(parentData: Any?): Any? =
        parentData ?: width.takeIf { it.isFinite() && it >= 0f }?.let(::CssPreferredWidth)
}

/** [width] uses layout pixels, as does a measured text advance. Preserve other parent contracts. */
fun Modifier.cssPreferredWidth(width: Float): Modifier = then(CssPreferredWidthModifier(width))

@Composable
fun CssFlowRow(
    modifier: Modifier = Modifier,
    horizontalGap: Dp = 0.dp,
    verticalGap: Dp = 0.dp,
    verticalAlignment: Alignment.Vertical = Alignment.Top,
    alignEnd: Boolean = false,
    content: @Composable () -> Unit,
) {
    Layout(content = content, modifier = modifier) { measurables, constraints ->
        val gapX = horizontalGap.toPx().coerceAtLeast(0f)
        val gapY = verticalGap.toPx().coerceAtLeast(0f)
        val childConstraints = constraints.copy(minWidth = 0, minHeight = 0)
        val children = measurables.map { it.measure(childConstraints) }
        val preferredWidths =
            measurables.mapIndexed { index, measurable ->
                val preferred = (measurable.parentData as? CssPreferredWidth)?.width
                (preferred ?: children[index].width.toFloat()).coerceIn(
                    0f,
                    if (constraints.hasBoundedWidth) constraints.maxWidth.toFloat()
                    else Float.MAX_VALUE,
                )
            }
        data class Position(val x: Float, val row: Int)
        data class RowInfo(val top: Float, val width: Float, val height: Int)
        val rows = ArrayList<RowInfo>()
        val positions = ArrayList<Position>(children.size)
        var rowWidth = 0f
        var rowHeight = 0
        var rowHasChildren = false
        var top = 0f
        var contentWidth = 0f
        children.forEachIndexed { index, child ->
            val width = preferredWidths[index]
            val next = if (rowHasChildren) rowWidth + gapX else 0f
            if (
                rowHasChildren && constraints.hasBoundedWidth && next + width > constraints.maxWidth
            ) {
                contentWidth = max(contentWidth, rowWidth)
                rows.add(RowInfo(top, rowWidth, rowHeight))
                top += rowHeight + gapY
                rowWidth = 0f
                rowHeight = 0
                rowHasChildren = false
            }
            val left = if (rowHasChildren) rowWidth + gapX else 0f
            positions.add(Position(left, rows.size))
            rowWidth = left + width
            rowHeight = max(rowHeight, child.height)
            rowHasChildren = true
        }
        if (rowHasChildren) rows.add(RowInfo(top, rowWidth, rowHeight))
        contentWidth = max(contentWidth, rowWidth)
        val contentHeight = if (rowHasChildren) top + rowHeight else 0f
        val width = constraints.constrainWidth(ceil(contentWidth.toDouble()).toInt())
        val height = constraints.constrainHeight(ceil(contentHeight.toDouble()).toInt())
        val horizontalDirection = if (layoutDirection == LayoutDirection.Rtl) -1f else 1f
        layout(width, height) {
            children.forEachIndexed { index, child ->
                val position = positions[index]
                val row = rows[position.row]
                val x = position.x + if (alignEnd) width - row.width else 0f
                val y = row.top + verticalAlignment.align(child.height, row.height)
                val left = floor(x).toInt()
                val topInt = floor(y).toInt()
                child.placeRelativeWithLayer(left, topInt) {
                    translationX = (x - left) * horizontalDirection
                    translationY = y - topInt
                }
            }
        }
    }
}
