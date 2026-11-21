package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/** Native field appearance. Text, composition and focus are owned by Field. */
@Composable
internal fun Modifier.androidInputDecoration(
    chrome: Boolean,
    focused: Boolean,
    invalid: Boolean,
    search: Boolean,
): Modifier {
    val colors = MaterialTheme.colorScheme
    val shape = RoundedCornerShape(if (search) 24.dp else AndroidDesign.surfaceRadius)
    val line = when {
        invalid -> colors.error
        focused -> colors.onSurfaceVariant
        chrome -> colors.outlineVariant
        else -> null
    }
    return then(if (chrome) Modifier.background(colors.surfaceVariant, shape) else Modifier)
        .then(if (line != null && (chrome || invalid)) Modifier.border(1.dp, line, shape) else Modifier)
}
