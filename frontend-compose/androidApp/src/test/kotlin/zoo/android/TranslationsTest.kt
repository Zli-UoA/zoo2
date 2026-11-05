package zoo.android

import kotlin.test.*
import kotlinx.serialization.json.*

class TranslationsTest {
    private val dictionary = Json.parseToJsonElement("""{"threads":{"replyCount_one":"{{count}} reply","replyCount_other":"{{count}} replies"},"plain":"Hello {{name}}"}""").jsonObject

    @Test fun resolvesPluralKeysAndInterpolatesArguments() {
        assertEquals("1 reply", translateDictionary(dictionary, "en", "threads.replyCount", mapOf("count" to "1")))
        assertEquals("0 replies", translateDictionary(dictionary, "en", "threads.replyCount", mapOf("count" to "0")))
        assertEquals("2 replies", translateDictionary(dictionary, "en", "threads.replyCount", mapOf("count" to "2")))
        assertEquals("1 replies", translateDictionary(dictionary, "ja", "threads.replyCount", mapOf("count" to "1")))
        assertEquals("Hello Zoo", translateDictionary(dictionary, "ja", "plain", mapOf("name" to "Zoo")))
        assertEquals("missing", translateDictionary(dictionary, "ja", "missing", emptyMap()))
    }
}
