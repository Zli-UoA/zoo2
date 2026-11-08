@file:OptIn(
    androidx.compose.ui.ExperimentalComposeUiApi::class,
    kotlin.js.ExperimentalWasmJsInterop::class,
)

package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import kotlin.JsFun
import kotlin.js.*
import kotlin.test.*
import kotlinx.coroutines.test.runTest
import org.jetbrains.compose.resources.decodeToImageBitmap

@JsFun(
    """async () => {
    for (let frames = 0; frames < 120; frames++) {
        await new Promise(resolve => requestAnimationFrame(resolve));
        if (frames >= 2 && (globalThis.__zooPendingImageDecodes ?? 0) === 0) {
            await new Promise(resolve => requestAnimationFrame(resolve));
            return 'ready';
        }
    }
    throw new Error('Icon image decode did not settle');
}"""
)
private external fun awaitIconImageDecode(): Promise<JsString>

class IconViewportSnapTest {
    private class Browser : Platform {
        var decoded = false
        var decodeCount = 0

        override suspend fun rasterizeSvg(svg: String, width: Int, height: Int): ByteArray {
            val bytes = browserRasterizeSvg(svg, width, height)
            decoded = true
            decodeCount++
            return bytes
        }

        override suspend fun headers() = emptyMap<String, String>()

        override suspend fun readCache(key: String): String? = null

        override suspend fun writeCache(key: String, value: String) {}

        override fun uuid() = "icon-snap-test"

        override fun now() = "2026-01-01T00:00:00Z"

        override fun navigate(path: String) {}

        override fun translate(key: String, args: Map<String, String>) = key

        override fun action(name: String) {}

        override fun time(timestamp: String) = timestamp

        override fun day(timestamp: String) = timestamp
    }

    @Test
    fun graphicsLayerCoordinatesRemainFractionalWhileSvgPaintSnapsToChromeViewport() = runTest {
        var origin = Offset.Unspecified
        val browser = Browser()
        var iconColor by mutableStateOf(Color.Black)
        val scene =
            ImageComposeScene(width = 24, height = 24, layoutDirection = LayoutDirection.Ltr) {
                CompositionLocalProvider(LocalWebPlatform provides browser) {
                    Box(Modifier.fillMaxSize().background(Color.White)) {
                        Box(
                            Modifier.size(20.dp).graphicsLayer {
                                translationX = .375f
                                translationY = .625f
                            }
                        ) {
                            Icon(
                                "bell",
                                modifier =
                                    Modifier.size(20.dp).onGloballyPositioned {
                                        origin = it.positionInRoot()
                                    },
                                color = iconColor,
                            )
                        }
                    }
                }
            }
        try {
            scene.render().close()
            assertEquals(
                .375f,
                origin.x,
                "Root coordinates include the ancestor's layer translation",
            )
            assertEquals(.625f, origin.y)
            var frame = 1L
            for (attempt in 0 until 8) {
                scene.render(frame++ * 16_000_000L).close()
                assertEquals("ready", awaitIconImageDecode().awaitBrowser().toString())
                if (browser.decoded) break
            }
            assertTrue(browser.decoded, "Exercise the browser PNG decoder, not the vector fallback")
            // Chrome's inline SVG paints at round(.375), round(.625), while its
            // bounding/client layout coordinates keep their fractional values.
            val paths = iconPaths.getValue("bell").joinToString("") { "<path d=\"$it\"/>" }
            val referenceSvg =
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\"><rect width=\"24\" height=\"24\" fill=\"white\"/><g transform=\"translate(0 1) scale(${20.0 / 24.0})\" fill=\"none\" stroke=\"#000000\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\">$paths</g></svg>"
            val expected = IntArray(24 * 24)
            browserRasterizeSvg(referenceSvg, 24, 24).decodeToImageBitmap().readPixels(expected)
            val image = scene.render(frame * 16_000_000L)
            try {
                val actual = IntArray(24 * 24)
                image.toComposeImageBitmap().readPixels(actual)
                assertContentEquals(
                    expected,
                    actual,
                    "Fractional SVG layout must retain Chrome's integer viewport raster",
                )
            } finally {
                image.close()
            }
            // Black preserves an exact coverage/geometry assertion above. Tinting
            // a white mask rounds colored antialiasing by at most one byte value.
            iconColor = Color(0xff656565)
            scene.render(++frame * 16_000_000L).close()
            awaitIconImageDecode().awaitBrowser()
            val grayReference = IntArray(24 * 24)
            browserRasterizeSvg(referenceSvg.replace("#000000", "#656565"), 24, 24)
                .decodeToImageBitmap().readPixels(grayReference)
            val grayImage = scene.render(++frame * 16_000_000L)
            try {
                val pixels = IntArray(24 * 24)
                grayImage.toComposeImageBitmap().readPixels(pixels)
                pixels.indices.forEach { index ->
                    for (shift in listOf(0, 8, 16, 24)) {
                        val delta = ((pixels[index] ushr shift) and 255) -
                            ((grayReference[index] ushr shift) and 255)
                        assertTrue(kotlin.math.abs(delta) <= 1, "Tint rounding at pixel $index")
                    }
                }
            } finally {
                grayImage.close()
            }
            val decodeCount = browser.decodeCount
            assertEquals(1, decodeCount, "Opaque RGB changes must reuse the raster")
            iconColor = Color.Red.copy(alpha = .5f)
            scene.render(++frame * 16_000_000L).close()
            awaitIconImageDecode().awaitBrowser()
            val recolored = scene.render(++frame * 16_000_000L)
            try {
                val pixels = IntArray(24 * 24)
                recolored.toComposeImageBitmap().readPixels(pixels)
                assertTrue(
                    pixels.any { pixel ->
                        val red = (pixel ushr 16) and 255
                        val green = (pixel ushr 8) and 255
                        red > green + 20 && green >= 127
                    },
                    "Tint must update both color and alpha",
                )
                assertEquals(decodeCount + 1, browser.decodeCount, "Keep SVG per-path opacity in the mask")
            } finally {
                recolored.close()
            }
            assertEquals(
                Offset(.375f, .625f),
                origin,
                "Paint snapping must not move layout/semantics",
            )
        } finally {
            scene.close()
        }
    }
}
