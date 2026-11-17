package zoo

import kotlin.test.*
import kotlinx.serialization.json.*

class ComponentsTest {
    private fun doc(text: String) = parseComponents(json.parseToJsonElement(text))

    @Test
    fun readsDocumentsAndDiscordNames() {
        val d = doc("""{"flags":32768,"components":[{"type":"container","components":[{"type":10,"content":"hi"}]},{"type":4242}]}""")!!
        assertTrue(d.hidesContent)
        assertEquals(listOf(17, 4242), d.components.map { componentType(it) })
        assertFalse(doc("""{"components":[{"type":10,"content":"x"}]}""")!!.hidesContent)
    }

    @Test
    fun brokenOrEmptyDocumentsFallBackToTheBody() {
        assertNull(parseComponents(null))
        assertNull(parseComponents(JsonPrimitive("not json")))
        assertNull(doc("""{"components":[]}"""))
        assertNull(doc("""[{"type":10}]"""))
        // 文字列で届いた JSON も読む（Electric の text 列）
        assertNotNull(parseComponents(JsonPrimitive("""{"components":[{"type":10,"content":"x"}]}""")))
    }

    @Test
    fun initialValuesFollowDefaultsAndMultiSelects() {
        val d = doc(
            """{"components":[
              {"type":1,"components":[{"type":3,"custom_id":"pick","max_values":2,"options":[{"label":"A","value":"a","default":true},{"label":"B","value":"b"}]}]},
              {"type":18,"label":"Name","component":{"type":4,"custom_id":"name","value":"x"}},
              {"type":18,"label":"Ok","component":{"type":23,"custom_id":"ok","default":true}},
              {"type":18,"label":"Plan","component":{"type":21,"custom_id":"plan","options":[{"label":"Free","value":"free","default":true}]}},
              {"type":1,"components":[{"type":2,"label":"Go","custom_id":"go"}]}]}"""
        )!!
        val values = initialComponentValues(d)
        assertEquals(JsonArray(listOf(JsonPrimitive("a"))), values["pick"])
        assertEquals(JsonPrimitive("x"), values["name"])
        assertEquals(JsonPrimitive(true), values["ok"])
        assertEquals(JsonPrimitive("free"), values["plan"])
        assertNull(values["go"], "buttons send their value only when pressed")
    }

    @Test
    fun onlyHttpsUrlsAreUsed() {
        assertEquals("https://example.com/a.png", safeHttps("https://example.com/a.png"))
        assertNull(safeHttps("http://example.com"))
        assertNull(safeHttps("javascript:alert(1)"))
        // 利用者情報付きの URL は使わない。パスの中の @ は使ってよい
        assertNull(safeHttps("https://user@evil.example/"))
        assertEquals("https://example.com/a@b", safeHttps("https://example.com/a@b"))
        assertNull(safeHttps("https://user@example.com/"))
    }

    @Test
    fun findsLabelsAnywhereInTheTree() {
        val nested = Json.parseToJsonElement(
            """[{"type":17,"components":[{"type":18,"label":"Name","component":{"type":4,"custom_id":"n"}}]}]"""
        )
        assertTrue(hasLabel(nested))
        assertFalse(hasLabel(Json.parseToJsonElement("""[{"type":10,"content":"x"}]""")))
    }
}
