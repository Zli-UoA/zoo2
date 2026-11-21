package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*

/** Android action chrome. The owning feature supplies the action and its current state. */
@Composable
internal fun AndroidAction(
    label: String,
    accessibleLabel: String,
    icon: String?,
    iconOnly: Boolean,
    enabled: Boolean,
    selected: Boolean?,
    expanded: Boolean?,
    variant: String,
    modifier: Modifier,
    role: Role,
    minimumTarget: Dp,
    glyphSize: Dp,
    inset: Dp,
    gap: Dp,
    textStyle: TextStyle?,
    foregroundOverride: Color = Color.Unspecified,
    onClick: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val activation = LocalControlActivation.current
    val interaction = remember { MutableInteractionSource() }
    val focused by interaction.collectIsFocusedAsState()
    val focusedByKeyboard = focused && FocusInputModality.keyboard
    val filled = variant in setOf("default", "destructive")
    val tone = when {
        !enabled -> colors.onSurfaceVariant.copy(alpha = if (filled) .75f else .5f)
        foregroundOverride != Color.Unspecified -> foregroundOverride
        textStyle?.color?.let { it != Color.Unspecified } == true -> textStyle.color
        variant == "default" -> colors.onPrimary
        variant == "destructive" -> colors.onError
        variant == "link" -> LocalLinkColor.current
        iconOnly && !filled -> colors.onSurfaceVariant
        else -> colors.onBackground
    }
    val fill = when {
        !enabled && filled -> colors.surfaceVariant
        variant == "default" -> colors.primary
        variant == "destructive" -> colors.error
        selected == true || expanded == true || variant == "secondary" -> colors.secondaryContainer
        else -> Color.Transparent
    }
    val line = when {
        focusedByKeyboard -> colors.onSurfaceVariant
        variant == "outline" -> colors.outline
        else -> Color.Transparent
    }
    Box(
        modifier.sizeIn(minWidth = minimumTarget, minHeight = minimumTarget)
            .clip(CircleShape)
            .focusInputModality()
            .clickable(interactionSource = interaction, indication = androidx.compose.foundation.LocalIndication.current,
                enabled = enabled, role = role) {
                activation?.invoke()
                onClick()
            }
            .semantics {
                contentDescription = accessibleLabel
                if (selected != null) this.selected = selected
                if (!enabled) disabled()
            },
        contentAlignment = Alignment.Center,
    ) {
        val height = if (filled || variant == "outline") 48.dp else AndroidDesign.controlVisualSize
        Row(
            Modifier.heightIn(min = height).then(if (iconOnly) Modifier.width(height) else Modifier)
                .background(fill, CircleShape)
                .then(if (line == Color.Transparent) Modifier else Modifier.border(1.dp, line, CircleShape))
                .padding(horizontal = if (iconOnly) 0.dp else inset, vertical = if (iconOnly) 0.dp else 4.dp),
            horizontalArrangement = if (iconOnly) Arrangement.Center else Arrangement.spacedBy(gap),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (icon != null) Icon(icon, modifier = Modifier.size(glyphSize), color = tone)
            if (!iconOnly) {
                CompositionLocalProvider(LocalMfmStyle provides LocalMfmStyle.current.merge(textStyle ?: TextStyle.Default)) {
                    Text(label, weight = textStyle?.fontWeight ?: FontWeight.Medium,
                        lineHeight = textStyle?.lineHeight ?: TextUnit.Unspecified,
                        size = textStyle?.fontSize?.takeIf { it.isSpecified } ?: AndroidDesign.buttonSize,
                        color = tone)
                }
            }
        }
    }
}
