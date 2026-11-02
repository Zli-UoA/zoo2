package zoo

import kotlin.test.*

class ReactionUsersTest {
    @Test
    fun labelsIncludeBothNamesAndDeduplicateActors() {
        val users =
            listOf(
                obj("id" to "alice", "display_name" to "Alice", "user_name" to "alice"),
                obj("id" to "bob", "user_name" to "bob"),
            )
        val rows =
            listOf(obj("user_id" to "alice"), obj("user_id" to "alice"), obj("user_id" to "bob"))
        assertEquals(
            listOf("Alice (@alice)", "@bob"),
            reactionUserLabels(rows, users, "unavailable"),
        )
    }

    @Test
    fun unresolvedActorsHaveOneStatusWithoutExposingIds() {
        val rows = listOf(obj("user_id" to "private-1"), obj("user_id" to "private-2"))
        assertEquals(listOf("loading"), reactionUserLabels(rows, emptyList(), "loading"))
        assertEquals(emptyList(), reactionUserLabels(emptyList(), emptyList(), "loading"))
    }
    @Test
    fun federatedActorsKeepTheirAccountNamesAlongsideLocalUsers() {
        val rows = listOf(obj("user_id" to "local"), obj("user_id" to "https://remote.example/users/peer", "display_name" to "Peer", "user_name" to "peer@remote.example"))
        val users = listOf(obj("id" to "local", "display_name" to "Local", "user_name" to "local"))
        assertEquals(listOf("Local (@local)", "Peer (@peer@remote.example)"), reactionUserLabels(rows, users, "unavailable"))
        assertEquals(":blobcat:", kitchenLabel("apub:${"a".repeat(64)}::blobcat:"))
    }
}
