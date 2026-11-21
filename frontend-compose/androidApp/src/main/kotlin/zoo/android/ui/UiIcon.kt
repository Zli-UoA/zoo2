package zoo.android.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

internal object AndroidControlSize {
    val target = 48.dp
    val inputEdgeInset = 4.dp
    val postActionInset = 8.dp
    val icon = 24.dp
    val navigationHeight = 60.dp
    val navigationIndicatorHeight = 32.dp
    val textInset = 24.dp
    val textVerticalInset = 8.dp
}

internal enum class UiIcon {
    Back, Next, Home, Channels, Bell, Bubble, Settings, Send, Attach, Format, Emoji, Create, Document, Copy, Phone, PhoneOff, Delete, Repost, Search,
}

@Composable
internal fun UiIconButton(
    icon: UiIcon,
    label: String,
    color: Color,
    action: () -> Unit,
    enabled: Boolean = true,
    fill: Color = Color.Transparent,
) {
    Box(
        Modifier.size(AndroidControlSize.target).clip(CircleShape).background(fill)
            .clickable(enabled = enabled, role = Role.Button, onClick = action)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        UiGlyph(icon, color)
    }
}

@Composable
internal fun UiGlyph(icon: UiIcon, color: Color) {
    Canvas(Modifier.size(AndroidControlSize.icon)) {
        fun point(x: Float, y: Float) = Offset(x * size.width / 24, y * size.height / 24)
        val width = 1.75.dp.toPx()
        fun lines(vararg points: Pair<Int, Int>) {
            val path = Path()
            points.forEachIndexed { index, (x, y) ->
                val p = point(x.toFloat(), y.toFloat())
                if (index == 0) path.moveTo(p.x, p.y) else path.lineTo(p.x, p.y)
            }
            drawPath(path, color, style = Stroke(width, cap = StrokeCap.Round, join = StrokeJoin.Round))
        }
        when (icon) {
            UiIcon.Back -> { lines(13 to 5, 6 to 12, 13 to 19); lines(6 to 12, 21 to 12) }
            UiIcon.Next -> lines(9 to 6, 15 to 12, 9 to 18)
            UiIcon.Home -> { lines(3 to 11, 12 to 3, 21 to 11); lines(5 to 10, 5 to 21, 19 to 21, 19 to 10) }
            UiIcon.Channels -> {
                lines(10 to 4, 7 to 20)
                lines(17 to 4, 14 to 20)
                lines(4 to 9, 21 to 9); lines(3 to 15, 20 to 15)
            }
            UiIcon.Bell -> { lines(5 to 17, 7 to 13, 7 to 8, 10 to 5, 14 to 5, 17 to 8, 17 to 13, 19 to 17, 5 to 17); lines(10 to 20, 14 to 20) }
            UiIcon.Bubble -> lines(4 to 5, 20 to 5, 20 to 17, 10 to 17, 5 to 21, 5 to 17, 4 to 17, 4 to 5)
            UiIcon.Settings -> {
                drawCircle(color, size.width * .32f, style = Stroke(width))
                drawCircle(color, size.width * .10f, style = Stroke(width))
                lines(12 to 2, 12 to 4); lines(12 to 20, 12 to 22)
                lines(2 to 12, 4 to 12); lines(20 to 12, 22 to 12)
            }
            UiIcon.Send -> { lines(3 to 4, 21 to 12, 3 to 20, 6 to 12, 3 to 4); lines(6 to 12, 21 to 12) }
            UiIcon.Attach -> lines(14 to 6, 7 to 13, 7 to 17, 10 to 20, 14 to 20, 21 to 13, 21 to 8, 18 to 5, 15 to 5, 9 to 11, 9 to 14, 11 to 16, 13 to 16, 18 to 11)
            UiIcon.Format -> { lines(4 to 19, 10 to 5, 16 to 19); lines(7 to 14, 13 to 14); lines(18 to 11, 21 to 11, 21 to 19, 18 to 19, 18 to 15, 21 to 15) }
            UiIcon.Emoji -> {
                drawCircle(color, size.width * .39f, style = Stroke(width))
                drawCircle(color, width / 2, point(9f, 9f))
                drawCircle(color, width / 2, point(15f, 9f))
                drawArc(color, 0f, 180f, false, point(8f, 11f), Size(size.width / 3, size.height / 4), style = Stroke(width))
            }
            UiIcon.Create -> lines(5 to 15, 16 to 4, 20 to 8, 9 to 19, 4 to 20, 5 to 15)
            UiIcon.Document -> {
                lines(6 to 3, 14 to 3, 19 to 8, 19 to 21, 6 to 21, 6 to 3)
                lines(14 to 3, 14 to 8, 19 to 8)
                lines(9 to 12, 16 to 12)
                lines(9 to 16, 16 to 16)
            }
            UiIcon.Search -> {
                drawCircle(color, radius = size.width * .24f, center = point(10f, 10f), style = Stroke(width))
                lines(14 to 14, 20 to 20)
            }
            UiIcon.Delete -> {
                lines(6 to 7, 18 to 7)
                lines(9 to 5, 15 to 5)
                lines(8 to 9, 9 to 19, 15 to 19, 16 to 9)
            }
            UiIcon.Repost -> {
                lines(5 to 9, 19 to 9, 16 to 6)
                lines(19 to 9, 16 to 12)
                lines(19 to 15, 5 to 15, 8 to 12)
                lines(5 to 15, 8 to 18)
            }
            UiIcon.Copy -> {
                lines(5 to 16, 3 to 16, 3 to 3, 16 to 3, 16 to 5)
                lines(8 to 8, 21 to 8, 21 to 21, 8 to 21, 8 to 8)
            }
            UiIcon.Phone -> lines(
                5 to 3, 9 to 3, 11 to 8, 8 to 10, 10 to 14, 14 to 16, 16 to 13,
                21 to 15, 21 to 19, 19 to 21, 13 to 20, 8 to 17, 4 to 12, 3 to 7, 5 to 3,
            )
            UiIcon.PhoneOff -> lines(
                3 to 12, 5 to 9, 9 to 7, 15 to 7, 19 to 9, 21 to 12, 21 to 16,
                16 to 16, 15 to 12, 9 to 12, 8 to 16, 3 to 16, 3 to 12,
            )
        }
    }
}
