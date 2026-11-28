package zoo.android

import java.io.File
import kotlin.test.*
import kotlinx.serialization.json.*

/** Android のコードが使う翻訳キーが、7 言語すべての辞書にあること（ないと日本語のまま、またはキーが出る）。 */
class TranslationKeysTest {
    private val root = generateSequence(File("").absoluteFile) { it.parentFile }.first { File(it, "frontend/src/lib/i18n/locales").isDirectory }
    private val locales = File(root, "frontend/src/lib/i18n/locales").listFiles { f -> f.extension == "json" }!!.associate {
        it.nameWithoutExtension to Json.parseToJsonElement(it.readText()).jsonObject
    }
    private val call = Regex("""(?:androidLabel|\.t|\bt|appText|phoneLabel\(\s*\w+\s*,)\(\s*"([A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+)"""")

    private fun has(dictionary: JsonObject, key: String): Boolean {
        fun lookup(path: String): JsonElement? {
            var value: JsonElement? = dictionary
            path.split('.').forEach { value = (value as? JsonObject)?.get(it) }
            return value
        }
        return lookup(key) is JsonPrimitive || lookup("${key}_other") is JsonPrimitive
    }

    @Test fun everyUsedKeyExistsInEveryLanguage() {
        assertEquals(setOf("ja", "en", "ko-KP", "ko-KR", "yue", "zh-Hans", "zh-Hant"), locales.keys)
        val sources = listOf("frontend-compose/androidApp/src/main/kotlin", "frontend-compose/src/commonMain/kotlin",
            "frontend-compose/src/androidMain/kotlin").flatMap { File(root, it).walkTopDown().filter { f -> f.extension == "kt" }.toList() }
        val used = sources.flatMap { file -> call.findAll(file.readText()).map { it.groupValues[1] to file.name } }.toSet()
        assertTrue(used.size > 100, "keys found: ${used.size}")
        val missing = used.flatMap { (key, file) ->
            locales.filterValues { !has(it, key) }.keys.map { "$key ($file): $it" }
        }.distinct().sorted()
        assertEquals(emptyList(), missing)
    }
}
