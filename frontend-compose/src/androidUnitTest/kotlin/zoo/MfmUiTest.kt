package zoo

import android.graphics.Bitmap
import android.graphics.Canvas
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.*
import java.io.File
import kotlin.math.roundToInt
import kotlin.test.*
import kotlinx.coroutines.*
import org.junit.Rule
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w800dp-h1400dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class MfmUiTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private class Fixture(reducedMotion: Boolean = true) {
        val actions = mutableListOf<String>()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            override fun uuid() = "mfm-test"
            override fun now() = "2026-11-16T00:00:00Z"
            override fun navigate(path: String) { actions += path }
            override fun action(name: String) { actions += name }
            override fun translate(key: String, args: Map<String, String>) = key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
        }
        val app = Session(
            Repository(zoo.Config("https://zoo.test", "/api/shapes", "alice", false), platform,
                HttpClient(MockEngine {
                    respond("""{"shortcodes":{"smile":"😀"},"data":[],"accounts":[]}""",
                        HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                }), scope, synchronize = false),
            scope, Environment(touch = true, reducedMotion = reducedMotion),
        )
        fun close() { app.close(); scope.cancel() }
    }

    @Composable
    private fun Shell(fixture: Fixture, dark: Boolean = false, content: @Composable () -> Unit) {
        CompositionLocalProvider(LocalApp provides fixture.app, LocalJoinedEmojis provides emptyList()) {
            MaterialTheme(colorScheme = zooColorScheme(dark)) {
                androidx.compose.material3.Surface(Modifier.fillMaxSize()) {
                    Box(Modifier.fillMaxSize()) { content() }
                }
            }
        }
    }

    @Test
    fun rubyRendersWholeStyledBaseAndSmallerReadingAboveIt() {
        val fixture = Fixture()
        try {
            ui.setContent {
                Shell(fixture) {
                    Box(Modifier.width(320.dp).testTag("ruby")) {
                        RichText("前 $[ruby **漢 字** かん じ] 後", selection = false)
                    }
                }
            }
            val base = ui.onNodeWithText("漢 字", useUnmergedTree = true)
            val reading = ui.onNodeWithText("かん じ", useUnmergedTree = true)
            base.assertIsDisplayed()
            reading.assertIsDisplayed()
            val baseLayout = layout(base)
            assertTrue(baseLayout.layoutInput.text.spanStyles.any { it.item.fontWeight == FontWeight.Bold })
            val readingLayout = layout(reading)
            assertTrue(readingLayout.layoutInput.style.fontSize < baseLayout.layoutInput.style.fontSize)
            assertTrue(reading.fetchSemanticsNode().boundsInRoot.bottom <= base.fetchSemanticsNode().boundsInRoot.top + 1f)
            val paragraph = ui.onNodeWithText("前 ", substring = true, useUnmergedTree = true)
            val paragraphBaseline = paragraph.fetchSemanticsNode().boundsInRoot.top + layout(paragraph).firstBaseline
            val baseBaseline = base.fetchSemanticsNode().boundsInRoot.top + baseLayout.firstBaseline
            assertTrue(kotlin.math.abs(paragraphBaseline - baseBaseline) <= 2f,
                "Ruby base is not aligned to its neighbors: $baseBaseline vs $paragraphBaseline")
            screenshot("ruby", "ruby-styled")
        } finally { fixture.close() }
    }

    @Test
    fun embeddedTextWrapsToItsContainerAcrossWidthsFontScalesAndThemes() {
        val fixture = Fixture()
        var width by mutableStateOf(320.dp)
        var scale by mutableStateOf(1f)
        var dark by mutableStateOf(false)
        val body = "長い日本語と English の本文です。".repeat(12)
        try {
            ui.setContent {
                Shell(fixture, dark) {
                    CompositionLocalProvider(LocalDensity provides Density(LocalDensity.current.density, scale)) {
                        Box(Modifier.width(width).testTag("wrapped")) {
                            RichText("先頭 $[fg.color=06f $body] 末尾", selection = false)
                        }
                    }
                }
            }
            for ((nextWidth, nextScale, nextDark) in listOf(
                Triple(320.dp, 1f, false), Triple(600.dp, 1f, true), Triple(320.dp, 1.8f, true),
            )) {
                ui.runOnIdle { width = nextWidth; scale = nextScale; dark = nextDark }
                val text = ui.onNodeWithText(body, useUnmergedTree = true)
                ui.mainClock.advanceTimeByFrame()
                try {
                    ui.waitUntil(timeoutMillis = 5_000) {
                        ui.onNodeWithTag("wrapped").fetchSemanticsNode().boundsInRoot.height >= layout(text).size.height
                    }
                } catch (failure: Throwable) {
                    val parent = ui.onNodeWithText("先頭", substring = true, useUnmergedTree = true)
                    val result = layout(parent)
                    println("MFM wrap $nextWidth/$nextScale: outer=${ui.onNodeWithTag("wrapped").fetchSemanticsNode().boundsInRoot}, " +
                        "paragraph=${result.size}, placeholders=${result.layoutInput.placeholders}, body=${layout(text).size}")
                    screenshot("wrapped", "wrap-failed")
                    throw failure
                }
                text.assertIsDisplayed()
                val textLayout = layout(text)
                assertTrue(textLayout.lineCount > 1)
                val outer = ui.onNodeWithTag("wrapped").fetchSemanticsNode().boundsInRoot
                val inner = text.fetchSemanticsNode().boundsInRoot
                assertTrue(inner.width <= outer.width + 1f, "$nextWidth / $nextScale: $inner outside $outer")
                assertTrue(outer.height >= textLayout.size.height, "The body is clipped: $outer vs ${textLayout.size}")
                assertTrue(inner.height >= textLayout.size.height - 1, "The body overlaps its placeholder")
                screenshot("wrapped", "wrap-${nextWidth.value}-${nextScale}-${nextDark}")
            }
        } finally { fixture.close() }
    }

    @Test
    fun everyFunctionRendersItsBodyWithoutLeakingItsMarkup() {
        val fixture = Fixture()
        var source by mutableStateOf("")
        try {
            ui.setContent {
                Shell(fixture) {
                    Box(Modifier.width(320.dp).testTag("function")) { RichText(source, selection = false) }
                }
            }
            for (name in supportedMfmFunctions) {
                val body = "probe-$name"
                ui.runOnIdle {
                    source = when (name) {
                        "ruby" -> "$[ruby $body reading]"
                        "unixtime" -> "$[unixtime 0]"
                        else -> "$[$name $body]"
                    }
                }
                val expected = if (name == "unixtime") "1970-01-01T00:00:00Z 1970-01-01T00:00:00Z" else body
                ui.onNodeWithText(expected, useUnmergedTree = true).assertIsDisplayed()
                screenshot("function", "function-$name")
            }
        } finally { fixture.close() }
    }

    @Test
    fun nestedEffectsInheritFontSizeColorAndWeight() {
        val fixture = Fixture()
        try {
            ui.setContent {
                Shell(fixture) {
                    Box(Modifier.width(320.dp)) {
                        RichText("**$[x2 $[fg.color=0f0 styled]]**", selection = false)
                    }
                }
            }
            val result = layout(ui.onNodeWithText("styled", useUnmergedTree = true))
            assertEquals(FontWeight.Bold, result.layoutInput.style.fontWeight)
            assertEquals(Color.Green, result.layoutInput.style.color)
            assertTrue(result.layoutInput.style.fontSize.value >= 28f)
        } finally { fixture.close() }
    }

    @Test
    fun clickableAndBlurRespondToTouchAndExposeTheirState() {
        val fixture = Fixture()
        try {
            ui.setContent {
                Shell(fixture) {
                    Column(Modifier.width(320.dp).testTag("interactive")) {
                        RichText("$[clickable.ev=demo tap]", selection = false)
                        RichText("$[blur secret]", selection = false)
                    }
                }
            }
            ui.onNode(hasText("tap") and hasClickAction()).performClick()
            ui.runOnIdle { assertEquals(listOf("mfm:demo"), fixture.actions) }
            val blur = ui.onNode(hasText("secret") and hasClickAction())
            blur.assertIsNotSelected()
            screenshot("interactive", "blur-before")
            blur.performTouchInput { click() }
            blur.assertIsSelected()
            screenshot("interactive", "blur-after")
        } finally { fixture.close() }
    }

    @Test
    fun blocksMathPlainAndCodeKeepTheirOwnBoundaries() {
        val fixture = Fixture()
        try {
            ui.setContent {
                Shell(fixture) {
                    Column(Modifier.width(320.dp).testTag("blocks")) {
                        RichText("# 見出し\n> 引用\n```kotlin\nval x = 1\n```\n<center>中央</center>\n<plain>**literal** $[spin x]</plain>\n\\(\\frac{1}{2}\\)", selection = false)
                    }
                }
            }
            for (text in listOf("見出し", "引用", "val x = 1\n", "中央")) {
                ui.onNodeWithText(text, useUnmergedTree = true).assertIsDisplayed()
            }
            ui.onNodeWithText("**literal** $[spin x]", substring = true, useUnmergedTree = true).assertIsDisplayed()
            ui.onNodeWithText("1", useUnmergedTree = true).assertIsDisplayed()
            ui.onNodeWithText("2", useUnmergedTree = true).assertIsDisplayed()
            screenshot("blocks", "blocks-and-math")
        } finally { fixture.close() }
    }

    @Test
    fun animatedFramesChangeWhileReducedMotionKeepsThemStill() {
        val fixture = Fixture(reducedMotion = false)
        var source by mutableStateOf("$[jelly.speed=1s animated]")
        ui.mainClock.autoAdvance = false
        try {
            ui.setContent {
                Shell(fixture) {
                    Box(Modifier.width(320.dp).height(160.dp).testTag("animation")) {
                        RichText(source, selection = false)
                    }
                }
            }
            ui.mainClock.advanceTimeByFrame()
            for (name in mfmAnimationDurations.keys) {
                ui.runOnIdle { source = "$[$name.speed=1s animated]" }
                ui.mainClock.advanceTimeByFrame()
                val target = ui.onNodeWithText(if (name == "sparkle") "✦" else "animated", useUnmergedTree = true)
                val beforeBounds = target.fetchSemanticsNode().boundsInRoot
                val beforeColor = layout(target).layoutInput.style.color
                ui.mainClock.advanceTimeBy(250)
                val afterBounds = target.fetchSemanticsNode().boundsInRoot
                val afterColor = layout(target).layoutInput.style.color
                if (name in setOf("rainbow", "sparkle")) assertNotEquals(beforeColor, afterColor, name)
                else assertNotEquals(beforeBounds, afterBounds, "$name did not move")
            }
            ui.runOnIdle { fixture.app.update(fixture.app.env.copy(reducedMotion = true)) }
            ui.mainClock.advanceTimeByFrame()
            val target = ui.onNodeWithText("✦", useUnmergedTree = true)
            val before = layout(target).layoutInput.style.color
            ui.mainClock.advanceTimeBy(500)
            assertEquals(before, layout(target).layoutInput.style.color)
        } finally { ui.mainClock.autoAdvance = true; fixture.close() }
    }

    @Test
    fun borderStylesAndFontFamiliesRenderWithTheirArguments() {
        val fixture = Fixture()
        var source by mutableStateOf("")
        try {
            ui.setContent {
                Shell(fixture) {
                    Box(Modifier.width(320.dp).testTag("variants")) { RichText(source, selection = false) }
                }
            }
            for (style in listOf("solid", "dotted", "dashed", "double", "groove", "ridge", "inset", "outset", "hidden")) {
                ui.runOnIdle { source = "$[border.width=4,style=$style,color=f00,radius=4 border]" }
                ui.onNodeWithText("border", useUnmergedTree = true).assertIsDisplayed()
                val bitmap = capture("variants")
                try {
                    val pixels = IntArray(bitmap.width * bitmap.height)
                    bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
                    val red = pixels.count { (it and 0x00ffffff) == 0x00ff0000 }
                    if (style == "hidden") assertEquals(0, red) else if (style in listOf("solid", "dotted", "dashed", "double"))
                        assertTrue(red > 0, "No red border for $style")
                } finally { bitmap.recycle() }
                screenshot("variants", "border-$style")
            }
            for (font in listOf("serif", "monospace", "cursive", "fantasy", "emoji", "math")) {
                ui.runOnIdle { source = "$[font.$font Font 日本語 😀]" }
                ui.onNodeWithText("Font 日本語 😀", useUnmergedTree = true).assertIsDisplayed()
                screenshot("variants", "font-$font")
            }
        } finally { fixture.close() }
    }

    @Test
    fun syntaxBlocksAndLiteralRegionsRenderWithoutInterpretation() {
        val fixture = Fixture()
        try {
            ui.setContent {
                Shell(fixture) {
                    Column(Modifier.width(320.dp).testTag("syntax")) {
                        RichText("# **見出し**", selection = false)
                        RichText("<center>中央寄せ</center>", selection = false)
                        RichText("> 引用1\n> 引用2", selection = false)
                        RichText("```kotlin\nval x = 1\n```", selection = false)
                        RichText("<plain>**literal** $[spin x]</plain>", selection = false)
                        RichText("\\(\\frac{a}{b}\\)", selection = false)
                        RichText("[**safe**](https://example.test/a_(b)) :smile:", selection = false)
                        RichText("__別の太字__ ***強調***", selection = false)
                    }
                }
            }
            ui.onNodeWithText("見出し", useUnmergedTree = true).assertIsDisplayed()
            ui.onNodeWithText("中央寄せ", useUnmergedTree = true).assertIsDisplayed()
            val centered = layout(ui.onNodeWithText("中央寄せ", useUnmergedTree = true))
            assertEquals(androidx.compose.ui.text.style.TextAlign.Center, centered.layoutInput.style.textAlign)
            val centerBounds = ui.onNodeWithText("中央寄せ", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
            val containerBounds = ui.onNodeWithTag("syntax").fetchSemanticsNode().boundsInRoot
            assertTrue(kotlin.math.abs(centerBounds.center.x - containerBounds.center.x) <= 1f,
                "The centered content is not visually centered")
            ui.onNodeWithText("引用1\n引用2", useUnmergedTree = true).assertIsDisplayed()
            ui.onNodeWithText("val x = 1\n", useUnmergedTree = true).assertIsDisplayed()
            ui.onNodeWithText("**literal** $[spin x]", useUnmergedTree = true).assertIsDisplayed()
            ui.onNodeWithText("a", useUnmergedTree = true).assertIsDisplayed()
            ui.onNodeWithText("b", useUnmergedTree = true).assertIsDisplayed()
            ui.onNodeWithContentDescription("safe").performTouchInput { click() }
            ui.runOnIdle { assertEquals(listOf("open:https://example.test/a_(b)"), fixture.actions) }
            screenshot("syntax", "syntax-blocks")
        } finally { fixture.close() }
    }

    private fun layout(node: SemanticsNodeInteraction): TextLayoutResult {
        val result = mutableListOf<TextLayoutResult>()
        node.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(result) }
        return result.single()
    }

    private fun capture(tag: String): Bitmap {
        val bounds = ui.onNodeWithTag(tag).fetchSemanticsNode().boundsInRoot
        var result: Bitmap? = null
        ui.runOnIdle {
            val view = ui.activity.findViewById<android.view.View>(android.R.id.content)
            val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
            try {
                view.draw(Canvas(full))
                result = Bitmap.createBitmap(full, bounds.left.roundToInt(), bounds.top.roundToInt(),
                    bounds.width.roundToInt().coerceAtLeast(1), bounds.height.roundToInt().coerceAtLeast(1))
                // createBitmap may return its input when the requested crop is the full image.
                if (result === full) result = full.copy(Bitmap.Config.ARGB_8888, false)
            } finally { full.recycle() }
        }
        return checkNotNull(result)
    }

    private fun screenshot(tag: String, name: String) {
        val directory = File(System.getProperty("zoo.uiScreenshotDir") ?: "build/reports/ui").apply { mkdirs() }
        val bitmap = capture(tag)
        try { File(directory, "mfm-$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) } }
        finally { bitmap.recycle() }
    }
}
