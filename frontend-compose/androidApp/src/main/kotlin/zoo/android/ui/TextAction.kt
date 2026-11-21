package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.sp

internal enum class TextActionStyle { Primary, Neutral, Plain }

@Composable
internal fun TextAction(
    title: String,
    colors: AndroidColors,
    enabled: Boolean = true,
    style: TextActionStyle = TextActionStyle.Primary,
    click: () -> Unit,
) {
    val fill = when (style) {
        TextActionStyle.Primary -> if (enabled) colors.accent else colors.inputBoundary
        TextActionStyle.Neutral -> colors.inputSurface
        TextActionStyle.Plain -> Color.Transparent
    }
    val ink = when {
        !enabled -> colors.inputHint
        style == TextActionStyle.Primary -> Color.Black
        else -> colors.foreground
    }
    Box(
        Modifier.heightIn(min = AndroidControlSize.target).clip(CircleShape).background(fill)
            .clickable(enabled = enabled, role = Role.Button, onClick = click)
            .padding(horizontal = AndroidControlSize.textInset, vertical = AndroidControlSize.textVerticalInset),
        contentAlignment = Alignment.Center,
    ) {
        Text(title, color = ink, fontSize = 15.sp, fontWeight = AndroidWeights.label)
    }
}
