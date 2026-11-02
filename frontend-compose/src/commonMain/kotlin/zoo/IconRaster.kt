package zoo

import androidx.compose.runtime.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.unit.IntSize
import kotlinx.coroutines.*
import org.jetbrains.compose.resources.decodeToImageBitmap

private data class IconRasterKey(val name: String, val argb: Int, val size: IntSize)

private val iconRasterScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
private val iconRasterCache = mutableMapOf<IconRasterKey, Deferred<ImageBitmap?>>()

@Composable
internal fun rememberIconRaster(name: String, color: Color, size: IntSize): ImageBitmap? {
    val platform = LocalWebPlatform.current
    val key = IconRasterKey(name, color.toArgb(), size)
    val bitmap by
        produceState<ImageBitmap?>(null, platform, key) {
            if (platform == null || size.width <= 0 || size.height <= 0) return@produceState
            value =
                iconRasterCache
                    .getOrPut(key) {
                        iconRasterScope.async {
                            val rgb = (key.argb and 0xffffff).toString(16).padStart(6, '0')
                            val opacity = ((key.argb ushr 24) and 255) / 255.0
                            val paths =
                                (iconPaths[name] ?: iconPaths.getValue("hash")).joinToString("") {
                                    "<path d=\"$it\"/>"
                                }
                            val svg =
                                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"${size.width}\" height=\"${size.height}\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"#$rgb\" stroke-opacity=\"$opacity\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\">$paths</svg>"
                            try {
                                platform
                                    .rasterizeSvg(svg, size.width, size.height)
                                    ?.decodeToImageBitmap()
                            } catch (cancelled: CancellationException) {
                                throw cancelled
                            } catch (_: Exception) {
                                null
                            }
                        }
                    }
                    .await()
        }
    return bitmap
}
