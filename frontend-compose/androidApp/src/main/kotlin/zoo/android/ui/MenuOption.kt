package zoo.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics

@Composable
internal fun MenuOption(label: String, colors: AndroidColors, icon: UiIcon? = null,
    chosen: Boolean? = null, click: () -> Unit) {
    DropdownMenuItem(
        text = { Text(label, color = colors.foreground,
            fontWeight = if (chosen == true) AndroidWeights.emphasis else AndroidWeights.body) },
        leadingIcon = icon?.let { { UiGlyph(it, colors.secondary, AndroidControlSize.inlineIcon) } },
        trailingIcon = chosen?.let { active -> {
            Box(Modifier.size(AndroidControlSize.inlineIcon)) {
                if (active) UiGlyph(UiIcon.Check, colors.foreground, AndroidControlSize.inlineIcon)
            }
        } },
        modifier = if (chosen == null) Modifier else Modifier.semantics { selected = chosen; role = Role.RadioButton },
        onClick = click,
    )
}
