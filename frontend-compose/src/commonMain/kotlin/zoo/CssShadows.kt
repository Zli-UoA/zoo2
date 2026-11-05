package zoo

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.dropShadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.shadow.Shadow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp

/** CSS blur is a Gaussian sigma of blur/2; Skia's legacy radius uses sigma=.57735r+.5. */
@Composable
private fun cssBlurRadius(blur: Dp): Dp =
    with(LocalDensity.current) { ((blur.toPx() / 2f - .5f).coerceAtLeast(0f) / .57735f).toDp() }

/** Tailwind's shadow-md uses two independent CSS box shadows, without tonal elevation. */
@Composable
fun Modifier.cssShadowMd(shape: Shape): Modifier =
    dropShadow(
            shape,
            Shadow(
                radius = cssBlurRadius(6.dp),
                spread = (-1).dp,
                color = Color.Black.copy(alpha = .1f),
                offset = DpOffset(0.dp, 4.dp),
            ),
        )
        .dropShadow(
            shape,
            Shadow(
                radius = cssBlurRadius(4.dp),
                spread = (-2).dp,
                color = Color.Black.copy(alpha = .1f),
                offset = DpOffset(0.dp, 2.dp),
            ),
        )

/** Sonner's default toast shadow. */
@Composable
fun Modifier.toastShadow(shape: Shape): Modifier =
    dropShadow(
        shape,
        Shadow(
            radius = cssBlurRadius(12.dp),
            color = Color.Black.copy(alpha = .1f),
            offset = DpOffset(0.dp, 4.dp),
        ),
    )

/** Tailwind's selected panel-tab shadow-sm. */
@Composable
fun Modifier.cssShadowSm(shape: Shape): Modifier =
    dropShadow(
            shape,
            Shadow(
                radius = cssBlurRadius(3.dp),
                color = Color.Black.copy(alpha = .1f),
                offset = DpOffset(0.dp, 1.dp),
            ),
        )
        .dropShadow(
            shape,
            Shadow(
                radius = cssBlurRadius(2.dp),
                spread = (-1).dp,
                color = Color.Black.copy(alpha = .1f),
                offset = DpOffset(0.dp, 1.dp),
            ),
        )
