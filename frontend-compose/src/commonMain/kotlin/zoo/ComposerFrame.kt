package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.*
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.dp

/** The shared frame owns only presentation; each composer retains its input and keyboard policy. */
@Composable
fun ComposerFrame(modifier: Modifier = Modifier, content: @Composable RowScope.() -> Unit) {
    val dark = MaterialTheme.colorScheme.background == Color.Black
    var focused by remember { mutableStateOf(false) }
    val ring = if (dark) Color(0xffbdbdbd) else Color(0xff737373)
    val input = if (dark) Color(0xff575757) else Color(0xffd4d4d4)
    Row(
        modifier
            .fillMaxWidth()
            .onFocusChanged { focused = it.hasFocus }
            .focusGroup()
            .drawBehind {
                if (focused)
                    drawRoundRect(
                        ring.copy(alpha = .3f),
                        Offset((-1.5).dp.toPx(), (-1.5).dp.toPx()),
                        Size(size.width + 3.dp.toPx(), size.height + 3.dp.toPx()),
                        CornerRadius(15.5.dp.toPx()),
                        style = Stroke(3.dp.toPx()),
                    )
            }
            .background(
                if (dark) input.copy(alpha = .3f) else MaterialTheme.colorScheme.background,
                RoundedCornerShape(14.dp),
            )
            .border(1.dp, if (focused) ring else input, RoundedCornerShape(14.dp))
            .padding(7.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.Bottom,
        content = content,
    )
}
