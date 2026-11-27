package zoo

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class MentionsTest {
    @Test
    fun searchesNormalizedNamesAndExcludesBotsThatCannotBeMentioned() {
        val users = listOf(
            obj("id" to "a", "user_name" to "alice", "display_name" to "ガリ"),
            obj("id" to "b", "user_name" to "bot", "display_name" to "ガリ", "is_bot" to true),
            obj("id" to "c", "user_name" to "app", "display_name" to "ガリ", "is_bot" to true, "bot_mentionable" to true),
            obj("id" to "d", "user_name" to "dave", "display_name" to "ガリ"),
        )
        val text = "本文（@カ\u3099リ）"
        val range = completionAt(text, text.lastIndex)!!
        assertEquals(listOf("alice", "app"), completionOptions(range, users, setOf("a", "b", "c")).map { it.name })
        assertEquals("本文（@alice ）" to 10, insertCompletion(text, range, "alice"))
        assertEquals("alice", completionAt("@ＡＬＩＣＥ", 6)?.query)
        assertNull(completionAt("mail@alice", 10))
        assertNull(completionAt("＠alice", 6))
    }

    @Test
    fun extractsEncryptedMentionsAndOnlyHighlightsVerifiedTargets() {
        val users = listOf(
            obj("id" to "2", "user_name" to "alice"),
            obj("id" to "1", "user_name" to "bob"),
        )
        val mentions = encryptedMentions("(@alice.) @bob、@alice @CHANNEL. @here! mail@unknown", users)
        assertEquals(MessageMentions(listOf("1", "2"), channel = true, here = true), mentions)
        val allowed = mentionAllowlist(mentions, users.take(1))
        assertTrue(allowed.allows("@alice."))
        assertTrue(allowed.allows("@CHANNEL"))
        assertTrue(allowed.allows("@here"))
        assertFalse(allowed.allows("@bob"))
        assertFalse(allowed.allows("@unknown"))
    }
}
