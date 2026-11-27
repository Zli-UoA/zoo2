package zoo

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/** `client-types` の `mentions::encrypted_mentions`、React の `encryptedMentions` と同じ例。 */
class EncryptedMentionsTest {
    private val users = listOf("u1" to "alice", "u2" to "bob.k", "u3" to "carol")

    @Test
    fun findsUsersChannelAndHereLikeTheServer() {
        val mentions = encryptedMentions("@alice と @bob.k. @Channel (@here) @nobody", users)
        assertEquals(listOf("u1", "u2"), mentions.userIds)
        assertTrue(mentions.channel)
        assertTrue(mentions.here)
    }

    @Test
    fun ignoresMailAddressesAndUnknownNames() {
        assertEquals(EncryptedMentions(), encryptedMentions("mail carol@example.com, @dave", users))
    }

    @Test
    fun deduplicatesAndSorts() {
        assertEquals(listOf("u1", "u3"), encryptedMentions("@carol @alice、@carol！", users).userIds)
    }
}
