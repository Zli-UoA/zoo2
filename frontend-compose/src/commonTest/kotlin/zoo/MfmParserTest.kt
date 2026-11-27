package zoo

import kotlin.test.*

class MfmParserTest {
    @Test
    fun registeredUnicodeAliasesDoNotConsumeUnknownShortcodesOrLiterals() {
        val alias = ":お祝い 猫:"
        val source = "時刻: $alias :unknown: `$alias` <plain>$alias</plain>"
        val tokens = parseInline(source, setOf(alias))
        assertEquals(listOf(alias, ":unknown:"), tokens.filter { it.kind == "emoji" }.map { it.value })
        assertEquals(listOf(alias, alias), tokens.filter { it.kind in setOf("code", "plain") }.map { it.value })
        assertTrue(alias in findCustomEmojiShortcodes(source))
    }

    @Test
    fun unclosedRecursiveSearchesFinishAndPreserveTheSource() {
        for (count in listOf(30, 40, 1000)) {
            val source = "$[a [ ".repeat(count)
            assertEquals(source, parseInline(source).joinToString("") { it.value })
            assertEquals(source, richSegments(source).joinToString("") { it.first })
        }
    }

    @Test
    fun headingsAfterProtectedRegionsUseTheOriginalNewlineConvention() {
        for (newline in listOf("\n", "\r\n", "\r")) {
            val source = listOf("<plain>", "# literal", "</plain>", "# End").joinToString(newline)
            assertEquals(MessageHeadingBlock("End", 1), messageHeadingBlocks(source).last())
            assertEquals(2, messageHeadingBlocks(source).size)
        }
    }

    @Test
    fun bracketsInLiteralRegionsAndLinksDoNotCloseFunctions() {
        for (body in listOf("`a]b`", "<plain>a]b</plain>", "\\(a]b\\)", "[docs](https://example.test/a_(b))", "$[fg.color=fff inner]")) {
            val source = "$[spin $body tail]"
            assertEquals(listOf(source to true), richSegments(source), body)
            assertEquals(listOf(Inline(source, "effect")), parseInline(source), body)
        }
    }

    @Test
    fun formattingRetainsEmbeddedFunctionsAsChildren() {
        val outer = parseInline("**before $[spin inner] after**").single()
        assertEquals("bold", outer.kind)
        assertEquals(listOf("text", "effect", "text"), parseInline(outer.value).map { it.kind })
        assertEquals("`x*y` tail", parseInline("*`x*y` tail*").single().value)
        assertEquals("outer **inner** end", parseInline("*outer **inner** end*").single().value)
        val nested = "outer $[fg.color=fff **inner**] end"
        assertEquals(Inline(nested, "bold"), parseInline("**$nested**").single())
    }

    @Test
    fun escapedSyntaxAndCodeRemainLiteral() {
        assertEquals(listOf(Inline("$[spin literal]")), parseInline("\\$[spin literal]"))
        assertEquals(listOf("\\$[spin literal]" to false), richSegments("\\$[spin literal]"))
        assertEquals(listOf(Inline("**bold**", "code")), parseInline("`**bold**`"))
        assertEquals(listOf(Inline("@alice $[spin x]", "plain")), parseInline("<plain>@alice $[spin x]</plain>"))
        assertEquals(listOf(Inline("a*b", "code")), parseInline("`a*b`"))
        assertEquals(listOf(Inline("foo_bar_baz")), parseInline("foo_bar_baz"))
    }

    @Test
    fun linksKeepBalancedParenthesesAndConsumeSilentPrefix() {
        val url = "https://example.test/a_(b)"
        assertEquals(listOf(Inline("docs", "link", url)), parseInline("?[docs]($url)"))
        assertFalse(parseInline("[bad](javascript:alert(1))").any { it.url != null })
        assertEquals(listOf(Inline("mail@example.test")), parseInline("mail@example.test"))
    }

    @Test
    fun onlyEmbeddedRegionsBecomeBlocksAndCentersBalance() {
        val source = "<center>one <center>two</center> three</center>"
        assertEquals(listOf(source to true), richSegments(source))
        assertEquals(listOf("> ordinary" to true), richSegments("> ordinary"))
        assertEquals(listOf("text > ordinary" to false), richSegments("text > ordinary"))
        assertEquals(listOf("<center>unclosed" to false), richSegments("<center>unclosed"))
    }

    @Test
    fun nestedTagsKeepTheirOwnClosingTags() {
        val token = parseInline("<b>one <b>two</b> three</b>").single()
        assertEquals(Inline("one <b>two</b> three", "bold"), token)
        assertEquals(listOf("text", "bold", "text"), parseInline(token.value).map { it.kind })
    }

    @Test
    fun malformedInputPreservesSource() {
        for (source in listOf("$[spin unclosed", "<plain>unclosed", "`unclosed", "**unclosed")) {
            assertEquals(source, richSegments(source).joinToString("") { it.first })
            assertEquals(source, parseInline(source).joinToString("") { it.value })
        }
    }

    @Test
    fun urlsLeaveSentencePunctuationOutsideBalancedDestinations() {
        assertEquals("https://example.test/a_(b)", parseInline("(https://example.test/a_(b)).")
            .single { it.kind == "link" }.url)
        assertEquals("https://example.test/a", parseInline("[https://example.test/a]")
            .single { it.kind == "link" }.url)
    }

    @Test
    fun fencedCodeRetainsItsBodyWithAndWithoutLanguageAndLineBreaks() {
        assertEquals(MfmCodeBlock("code", ""), mfmCodeBlock("```code```"))
        assertEquals(MfmCodeBlock("val x = 1\n", "kotlin"), mfmCodeBlock("```kotlin\nval x = 1\n```"))
        assertEquals(MfmCodeBlock("code", ""), mfmCodeBlock("```\ncode```"))
    }

    @Test
    fun multilineInlineStylesProtectHeadingLikeContent() {
        assertTrue(messageHeadingBlocks("**bold\n# literal\nend**").all { it.level == null })
    }

    @Test
    fun everyTruncationOfNestedInputRemainsParseableWithoutSourceLoss() {
        val source = "$[ruby **base $[fg.color=f00 `a]b`]** reading]\n<center>x</center>"
        for (length in 0..source.length) {
            val prefix = source.take(length)
            assertEquals(prefix, richSegments(prefix).joinToString("") { it.first })
            parseInline(prefix)
        }
    }

    @Test
    fun upstreamBoldAndTadaShorthandsHaveTheirMfmMeaning() {
        assertEquals(listOf(Inline("bold", "bold")), parseInline("__bold__"))
        assertEquals(listOf(Inline("https://example.test", "link", "https://example.test", bare = true)),
            parseInline("<https://example.test>"))
        assertEquals(listOf(Inline("$[tada emphasis]", "effect")), parseInline("***emphasis***"))
    }

    @Test
    fun ordinaryOpeningBracketsDoNotIntroduceAnMfmSubtree() {
        assertEquals(listOf("$[spin a[b]" to true, " c]" to false), richSegments("$[spin a[b] c]"))
        assertEquals(listOf("$[spin [docs](https://example.test) tail]" to true),
            richSegments("$[spin [docs](https://example.test) tail]"))
    }
}
