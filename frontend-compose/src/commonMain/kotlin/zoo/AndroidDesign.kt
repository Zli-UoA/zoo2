package zoo

import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Android layout contract. See docs/android-design.md before adding a new size. */
internal object AndroidDesign {
    val spaceSmall = 4.dp
    val spaceMedium = 8.dp
    val spaceLarge = 12.dp
    val screenInset = 16.dp
    val sectionGap = 24.dp

    val touchTarget = 48.dp
    val headerMinHeight = 56.dp
    val icon = 24.dp
    val controlVisualSize = 32.dp
    val avatar = 36.dp
    val controlRadius = 8.dp
    val surfaceRadius = 12.dp

    // Dense feeds share a keyline; settings and forms use screenInset instead.
    val feedInset = spaceLarge
    val postVerticalInset = spaceLarge
    val controlInset = spaceLarge
    val controlGap = spaceMedium
    val avatarGap = spaceLarge
    val headerInset = feedInset
    val settingsRowInset = screenInset

    val bodySize = 17.sp
    val bodyLineHeight = 25.sp
    val supportingSize = 14.sp
    val supportingLineHeight = 20.sp
    val inputSize = 16.sp
    val inputLineHeight = 24.sp
    val channelTitleSize = 16.sp
    val channelDescriptionSize = 13.sp
    val titleSize = 20.sp
    val titleLineHeight = 28.sp
    val labelSize = 12.sp
    val buttonSize = 15.sp
}
