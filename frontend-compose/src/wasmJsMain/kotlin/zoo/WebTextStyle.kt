package zoo

import androidx.compose.ui.text.*

/** Match browser glyph advance rounding instead of the desktop Skia default. */
@OptIn(ExperimentalTextApi::class)
internal actual fun webTextStyle() =
    PlatformTextStyle(
        spanStyle = null,
        paragraphStyle =
            PlatformParagraphStyle(
                FontRasterizationSettings(
                    smoothing = FontSmoothing.AntiAlias,
                    hinting = FontHinting.Slight,
                    subpixelPositioning = true,
                    autoHintingForced = false,
                )
            ),
    )
