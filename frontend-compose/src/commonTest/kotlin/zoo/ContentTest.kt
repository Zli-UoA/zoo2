package zoo

import kotlin.test.*
import kotlinx.serialization.json.*

class ContentTest {
    @Test
    fun authzAvatarShortcodesAreDistinctFromMentionsAndPreserveCode() {
        val tokens = parseInline(":@alice: @alice `:@alice: @alice` <plain>:@alice:</plain>")
        assertEquals(1, tokens.count { it.kind == "accountEmoji" })
        assertEquals(1, tokens.count { it.kind == "mention" })
        assertEquals(":@alice: @alice", tokens.single { it.kind == "code" }.value)
        assertEquals(":@alice:", tokens.single { it.kind == "plain" }.value)
    }

    @Test
    fun groupingDoesNotMergeMentionsWithReactions() {
        fun notification(id: String, kind: String, actor: String) =
            obj(
                "id" to id,
                "notification_type" to kind,
                "message_id" to "m",
                "channel_id" to "c",
                "user_id" to "me",
                "actor_id" to actor,
            )
        val groups =
            notificationGroups(
                listOf(
                    notification("a", "reaction", "alice"),
                    notification("b", "reaction", "bob"),
                    notification("c", "mention", "alice"),
                )
            )
        assertEquals(listOf(2, 1), groups.map { it.size }.sortedDescending())
    }

    @Test
    fun inlineCodeIsNotParsedAsFormattingOrLinks() {
        assertEquals(
            listOf(Inline("**https://bad.test**", "code")),
            parseInline("`**https://bad.test**`"),
        )
        assertEquals(
            "https://example.test",
            parseInline("[safe](https://example.test)").single().url,
        )
        assertFalse(parseInline("[unsafe](javascript:alert(1))").any { it.url != null })
    }

    @Test
    fun segmentScannerKeepsCodeAndPlainLiteralAndSeparatesQuotes() {
        assertEquals(listOf("`$[spin literal]`" to false), richSegments("`$[spin literal]`"))
        assertEquals(
            listOf("<plain>$[spin literal]</plain>" to true),
            richSegments("<plain>$[spin literal]</plain>"),
        )
        assertEquals(
            listOf("before\n" to false, "> quote\n> next" to true, "\nafter" to false),
            richSegments("before\n> quote\n> next\nafter"),
        )
        assertEquals(
            listOf(Inline("**literal**", "plain")),
            parseInline("<plain>**literal**</plain>"),
        )
        assertEquals(listOf(Inline("small", "small")), parseInline("<small>small</small>"))
        assertEquals(
            listOf(Inline("@alice@example.com", "mention"), Inline(" "), Inline("#開発", "hashtag")),
            parseInline("@alice@example.com #開発"),
        )
    }

    @Test
    fun webhookListsReadTheActualApiEnvelope() {
        val rows = JsonArray(listOf(obj("id" to "hook", "name" to "Webhook")))
        assertEquals(
            rows.rows(),
            buildJsonObject {
                    put("data", rows)
                    put("txid", JsonNull)
                }
                .rows(),
        )
        assertEquals("hook", rows.rows().single().text("id"))
        assertTrue(obj("data" to "unrelated").rows().isEmpty())
    }

    @Test
    fun completionHonorsCodeAndChannelMembershipAndKeepsTheCaretInsideMfm() {
        assertNull(completionAt("`$[sp", 5))
        assertNull(completionAt("https://example.test/\$sp", 23))
        val range = completionAt("hello $[sp", 10)!!
        val (value, caret) = insertCompletion("hello $[sp", range, "spin")
        assertEquals("hello $[spin ]", value)
        assertEquals(value.lastIndex, caret)
        val users =
            listOf(obj("id" to "a", "user_name" to "alice"), obj("id" to "b", "user_name" to "bob"))
        assertEquals(
            listOf("alice"),
            completionOptions(CompletionRange(0, 2, "a", false), users, setOf("a"))
                .map { it.name }
                .filter { it !in listOf("channel", "here") },
        )
    }

    @Test
    fun completionLeavesDeactivatedPlaceholdersOut() {
        val users =
            listOf(
                obj("id" to "a", "user_name" to "alice"),
                obj("id" to "s", "user_name" to "alice_slack", "is_deactivated" to true),
            )
        assertEquals(
            listOf("alice"),
            completionOptions(CompletionRange(0, 2, "alice", false), users, setOf("a", "s"))
                .map { it.name },
        )
    }
}
