@file:OptIn(
    kotlin.js.ExperimentalWasmJsInterop::class,
    kotlin.io.encoding.ExperimentalEncodingApi::class,
    androidx.compose.ui.text.ExperimentalTextApi::class,
    org.jetbrains.compose.resources.ExperimentalResourceApi::class,
)

package zoo

import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.*
import androidx.compose.ui.text.platform.Font
import androidx.compose.ui.unit.*
import kotlin.JsFun
import kotlin.io.encoding.Base64
import kotlin.js.*
import kotlin.test.*
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*
import org.jetbrains.compose.resources.getFontResourceBytes
import org.jetbrains.compose.resources.getSystemResourceEnvironment
import zoo_compose.generated.resources.*

@JsFun(
    """async (encoded, samples) => {
    const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
    const font = new FontFace('ZooFontMetrics', bytes.buffer, {weight: '100 900'});
    await font.load();
    document.fonts.add(font);
    try {
        const context = document.createElement('canvas').getContext('2d');
        context.font = '500 14px "ZooFontMetrics"';
        return JSON.stringify(JSON.parse(samples).map(text => context.measureText(text).width));
    } finally {
        document.fonts.delete(font);
    }
}"""
)
private external fun browserFontWidths(encoded: String, samples: String): Promise<JsString>

/** Keeps native Latin text advances aligned with the browser using the same font bytes. */
class FontMetricsTest {
    @Test
    fun geistMediumAdvancesMatchBrowserIncludingKerning() = runTest {
        val bytes = getFontResourceBytes(getSystemResourceEnvironment(), Res.font.Geist)
        assertTrue(bytes.isNotEmpty(), "Measure the actual bundled Geist font")
        val words =
            listOf(
                "default",
                "outline",
                "secondary",
                "ghost",
                "destructive",
                "link",
                "AVATAR",
                "office",
                "fi fl",
                "To Wa",
            )
        val browser =
            Json.parseToJsonElement(
                    browserFontWidths(
                            Base64.encode(bytes),
                            JsonArray(words.map(::JsonPrimitive)).toString(),
                        )
                        .awaitBrowser()
                        .toString()
                )
                .jsonArray
                .map { it.jsonPrimitive.double }
        assertEquals(words.size, browser.size)
        // resources.Font(Res.font.Geist, Medium) derives these variation axes.
        val family =
            FontFamily(
                Font(
                    "font-metrics-geist-medium",
                    bytes,
                    FontWeight.Medium,
                    FontStyle.Normal,
                    FontVariation.Settings(FontWeight.Medium, FontStyle.Normal),
                )
            )
        val measurer = TextMeasurer(createFontFamilyResolver(), Density(1f), LayoutDirection.Ltr, 0)
        val style =
            TextStyle(
                fontFamily = family,
                fontWeight = FontWeight.Medium,
                fontSize = 14.sp,
                platformStyle = webTextStyle(),
            )
        words.forEachIndexed { index, word ->
            val native = measurer.measure(word, style, softWrap = false).getLineRight(0).toDouble()
            assertTrue(browser[index].isFinite() && browser[index] > 0)
            assertEquals(browser[index], native, "Glyph advance for '$word'")
        }
    }

    @Test
    fun explicitCssLineHeightsDoNotAccumulateRoundingPixels() = runTest {
        val bytes = getFontResourceBytes(getSystemResourceEnvironment(), Res.font.NotoSansJPRegular)
        val family =
            FontFamily(
                Font(
                    "line-height-japanese",
                    bytes,
                    FontWeight.Normal,
                    FontStyle.Normal,
                    FontVariation.Settings(FontWeight.Normal, FontStyle.Normal),
                )
            )
        val measurer = TextMeasurer(createFontFamilyResolver(), Density(1f), LayoutDirection.Ltr, 0)
        for (height in listOf(16, 20, 24)) {
            val result =
                measurer.measure(
                    "行間\n行間\n行間",
                    TextStyle(
                        fontFamily = family,
                        fontSize = 14.sp,
                        lineHeight = browserLineHeight(height.sp),
                        platformStyle = webTextStyle(),
                        lineHeightStyle =
                            androidx.compose.ui.text.style.LineHeightStyle(
                                androidx.compose.ui.text.style.LineHeightStyle.Alignment.Center,
                                androidx.compose.ui.text.style.LineHeightStyle.Trim.None,
                            ),
                    ),
                    softWrap = false,
                )
            assertEquals(
                height.toFloat(),
                result.getLineTop(1) - result.getLineTop(0),
                "first line at $height",
            )
            assertEquals(
                height.toFloat(),
                result.getLineTop(2) - result.getLineTop(1),
                "second line at $height",
            )
        }
    }

    @Test
    fun mixedFallbackFontsKeepTheCssLineBoxAndPrimaryBaseline() = runTest {
        val environment = getSystemResourceEnvironment()
        val latinBytes = getFontResourceBytes(environment, Res.font.Geist)
        val cjkBytes = getFontResourceBytes(environment, Res.font.NotoSansJPRegular)
        val latin =
            FontFamily(
                Font(
                    "linebox-geist-medium",
                    latinBytes,
                    FontWeight.Medium,
                    FontStyle.Normal,
                    FontVariation.Settings(FontWeight.Medium, FontStyle.Normal),
                )
            )
        val cjk =
            FontFamily(
                Font(
                    "linebox-cjk-medium",
                    cjkBytes,
                    FontWeight.Medium,
                    FontStyle.Normal,
                    FontVariation.Settings(FontWeight.Normal, FontStyle.Normal),
                )
            )
        val fonts = Fonts(latin, cjk, latin)
        val measurer = TextMeasurer(createFontFamilyResolver(), Density(1f), LayoutDirection.Ltr, 0)
        val style =
            TextStyle(
                fontFamily = latin,
                fontWeight = FontWeight.Medium,
                fontSize = 14.sp,
                lineHeight = browserLineHeight(20.sp),
                platformStyle = webTextStyle(),
                lineHeightStyle =
                    androidx.compose.ui.text.style.LineHeightStyle(
                        androidx.compose.ui.text.style.LineHeightStyle.Alignment.Center,
                        androidx.compose.ui.text.style.LineHeightStyle.Trim.None,
                    ),
            )
        val primary = measurer.measure(fontText("Bob", fonts, FontWeight.Medium), style)
        val mixed = measurer.measure(fontText("Bob さんほか1人", fonts, FontWeight.Medium), style)
        assertEquals(20, mixed.size.height)
        assertEquals(primary.firstBaseline, mixed.firstBaseline)
        val multiline =
            measurer.measure(
                fontText("Bob さんほか1人\n日本語 ABC\n田中 ゆきこ", fonts, FontWeight.Medium),
                style,
            )
        assertEquals(3, multiline.lineCount)
        assertEquals(60, multiline.size.height)
        assertEquals(20f, multiline.getLineBaseline(1) - multiline.firstBaseline, .0001f)
        assertEquals(40f, multiline.getLineBaseline(2) - multiline.firstBaseline, .0001f)
    }
}
