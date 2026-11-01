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
}
