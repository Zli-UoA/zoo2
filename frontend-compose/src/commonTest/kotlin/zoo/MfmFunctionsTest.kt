package zoo

import kotlin.math.abs
import kotlin.test.*

class MfmFunctionsTest {
    @Test
    fun everyAdvertisedFunctionHasAValidatedDefinition() {
        val expected = setOf(
            "tada", "jelly", "twitch", "shake", "spin", "jump", "bounce", "flip",
            "x2", "x3", "x4", "scale", "position", "fg", "bg", "border", "font", "blur",
            "rainbow", "sparkle", "rotate", "ruby", "unixtime", "clickable",
        )
        assertEquals(expected, supportedMfmFunctions)
        for (name in expected) {
            assertEquals(name, mfmFunction("$name body").name)
            assertEquals("body", mfmFunction("$name body").body)
            assertEquals("effect", parseInline("$[$name body]").single().kind)
        }
    }

    @Test
    fun headersRetainWhitespaceAndParseFlagsAndDecimalArguments() {
        val fn = mfmFunction("scale.x=-1.5,y=.25,flag\n body with spaces")
        assertEquals(mapOf("x" to "-1.5", "y" to ".25", "flag" to "true"), fn.args)
        assertEquals(" body with spaces", fn.body)
        assertEquals("body", mfmFunction("ruby\tbody").body)
    }

    @Test
    fun durationsRequireUnitsAndRejectNonFiniteOrInjectedValues() {
        assertEquals(.5f, mfmFunction("spin.speed=1ms x").seconds("speed", 1.5f))
        assertEquals(2.5f, mfmFunction("spin.speed=2500ms x").seconds("speed", 1.5f))
        assertEquals(86400f, mfmFunction("spin.speed=999999s x").seconds("speed", 1.5f))
        assertEquals(-86400f, mfmFunction("spin.delay=-999999s x").seconds("delay", 0f, true))
        for (raw in listOf("NaN", "Infinity", "1", "1e3s", "0s;display:none", "-Infinitys")) {
            assertEquals(1.5f, mfmFunction("spin.speed=$raw x").seconds("speed", 1.5f), raw)
        }
        assertEquals(-.25f, mfmFunction("spin.delay=-250ms x").seconds("delay", 0f, true))
    }

    @Test
    fun colorsPreserveRgbaForAllSupportedLengths() {
        for ((raw, argb) in mapOf(
            "f00" to 0xffff0000L, "0f08" to 0x8800ff00L,
            "aAbBcC" to 0xffaabbccL, "11223344" to 0x44112233L,
        )) assertEquals(argb, mfmFunction("fg.color=$raw x").argb("color"))
        for (raw in listOf("ff", "fffff", "#fff", "-fff", "xyz", "transparent")) {
            assertNull(mfmFunction("fg.color=$raw x").argb("color"), raw)
        }
    }

    @Test
    fun scalingPreservesAspectRatioAndNeverRecoversBudgetByShrinking() {
        val anisotropic = mfmFunction("scale.x=5,y=-1 x").scale(4f)
        assertEquals(2f, anisotropic.x)
        assertEquals(-.4f, anisotropic.y)
        assertEquals(8f, anisotropic.cumulative)
        var cumulative = 1f
        var product = 1f
        for (source in listOf("scale.x=5,y=-5", "scale.x=.1,y=.1", "x4", "scale.x=5,y=1")) {
            val next = mfmFunction("$source x").scale(cumulative)
            assertTrue(next.cumulative >= cumulative)
            product *= maxOf(1f, abs(next.x), abs(next.y))
            assertTrue(product <= 8.001f)
            cumulative = next.cumulative
        }
        assertEquals(8f, cumulative)
    }

    @Test
    fun transformsUseDocumentedBoundsAndFlags() {
        assertEquals(MfmMotion(scaleX = -1f), mfmFunction("flip x").motion(0f, false, 1f))
        assertEquals(MfmMotion(scaleY = -1f), mfmFunction("flip.v x").motion(0f, false, 1f))
        assertEquals(MfmMotion(scaleX = -1f, scaleY = -1f), mfmFunction("flip.h,v x").motion(0f, false, 1f))
        assertEquals(36000f, mfmFunction("rotate.deg=99999 x").motion(0f, false, 1f).rotationZ)
        assertEquals(MfmMotion(x = -2f, y = 2f), mfmFunction("position.x=-100,y=100 x").motion(0f, false, 1f))
        assertEquals(-90f, mfmFunction("spin.left,x x").motion(.25f, false, 1f).rotationX)
        assertEquals(90f, mfmFunction("spin.y x").motion(.25f, false, 1f).rotationY)
        assertEquals(1f, mfmFunction("scale.x=NaN x").scale(1f).x)
    }

    @Test
    fun everyAnimationIsStationaryWithReducedMotionAndFiniteAtEachFrame() {
        for (name in mfmAnimationDurations.keys) {
            for (phase in listOf(0f, .125f, .25f, .5f, 1f)) {
                val fn = mfmFunction("$name x")
                assertEquals(MfmMotion(), fn.motion(phase, true, 1f), "$name $phase")
                val frame = fn.motion(phase, false, 1f)
                assertTrue(listOf(frame.x, frame.y, frame.scaleX, frame.scaleY, frame.rotationZ).all { it.isFinite() })
            }
        }
        assertEquals(MfmMotion(), mfmFunction("shake x").motion(0f, false, 1f))
    }

    @Test
    fun rubySplitsOnlyRootTextAndPreservesCompleteBaseMarkup() {
        val cases = mapOf(
            "漢字 かんじ" to MfmRuby("漢字", "かんじ"),
            "**漢 字** かん じ" to MfmRuby("**漢 字**", "かん じ"),
            "$[fg.color=f00 漢 字] かんじ" to MfmRuby("$[fg.color=f00 漢 字]", "かんじ"),
            "[漢字](https://example.test/a_(b)) かんじ" to MfmRuby("[漢字](https://example.test/a_(b))", "かんじ"),
            "`漢 字` かんじ" to MfmRuby("`漢 字`", "かんじ"),
            "<plain>漢 字</plain> かんじ" to MfmRuby("<plain>漢 字</plain>", "かんじ"),
            "**漢字**" to MfmRuby("**漢字**", ""),
            "New **York** よーく" to MfmRuby("New **York**", "よーく"),
            "**星** \\*" to MfmRuby("**星**", "*"),
            "漢字\tかんじ" to MfmRuby("漢字", "かんじ"),
            "" to MfmRuby("", ""),
        )
        for ((source, expected) in cases) assertEquals(expected, mfmRuby(source), source)
    }

    @Test
    fun unixTimeAcceptsIntegerSecondsWithinTheInteroperableRange() {
        assertEquals(0L, mfmUnixSeconds(" 0 "))
        assertEquals(-1L, mfmUnixSeconds("-1"))
        assertEquals(8_640_000_000_000L, mfmUnixSeconds("8640000000000"))
        for (value in listOf("+1", "1.5", "NaN", "1e3", "8640000000001", "9223372036854775807"))
            assertNull(mfmUnixSeconds(value), value)
    }

    @Test
    fun fontWithoutARecognizedFlagInheritsItsParent() {
        assertNull(mfmFont(emptyMap()))
        assertNull(mfmFont(mapOf("unknown" to "true")))
        for (font in listOf("serif", "monospace", "cursive", "fantasy", "emoji", "math"))
            assertEquals(font, mfmFont(mapOf(font to "true")))
        assertEquals("serif", mfmFont(mapOf("monospace" to "true", "serif" to "true")))
    }
}
