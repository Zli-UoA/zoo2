package zoo

import kotlin.test.*

class MathParserTest {
    @Test
    fun fractionsScriptsRootsAndMatricesHaveStructuredNodes() {
        val fraction = (MathParser("\\frac{a}{b}").parse() as MathNode.Group).values.single()
        assertIs<MathNode.Fraction>(fraction)
        val script = (MathParser("x_i^2").parse() as MathNode.Group).values.single() as MathNode.Script
        assertEquals(MathNode.Literal("x"), script.base)
        assertEquals(MathNode.Literal("i"), script.subscript)
        assertEquals(MathNode.Literal("2"), script.superscript)
        assertIs<MathNode.Root>((MathParser("\\sqrt{x}").parse() as MathNode.Group).values.single())
        val matrix = (MathParser("\\begin{pmatrix}a&b\\\\c&d\\end{pmatrix}").parse() as MathNode.Group)
            .values.single() as MathNode.Matrix
        assertEquals("(" to ")", matrix.brackets)
        assertEquals(listOf(2, 2), matrix.rows.map { it.size })
    }

    @Test
    fun invalidOrUnsupportedFormulasRetainTheirSource() {
        for (source in listOf("\\unknown{x}", "\\frac{x}", "{x", "x}", "x^", "\\sqrt")) {
            assertEquals(MathNode.Literal(source), MathParser(source).parse(), source)
        }
    }

    @Test
    fun unbracedCommandChainsRespectTheDepthLimit() {
        for (command in listOf("\\sqrt", "\\frac", "\\hat", "\\mathrm")) {
            val source = command.repeat(5000)
            assertEquals(MathNode.Literal(source), MathParser(source).parse())
        }
    }

    @Test
    fun symbolAndEscapedPunctuationRemainLiteral() {
        val nodes = (MathParser("\\alpha\\{\\%\\}").parse() as MathNode.Group).values
        assertEquals(listOf("α", "{", "%", "}"), nodes.map { (it as MathNode.Literal).value })
    }

    @Test
    fun deeplyNestedInputDoesNotOverflowTheStack() {
        MathParser("{".repeat(1000) + "x" + "}".repeat(1000)).parse()
    }
}
