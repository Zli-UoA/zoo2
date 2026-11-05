package zoo.android

import java.util.Locale
import kotlinx.serialization.json.*
import zoo.Row

internal fun translateDictionary(dictionary: Row, locale: String, key: String, args: Map<String, String>): String {
    fun lookup(path: String): String? {
        var value: JsonElement? = dictionary
        path.split('.').forEach { value = (value as? JsonObject)?.get(it) }
        return (value as? JsonPrimitive)?.content
    }
    val count = args["count"]?.toDoubleOrNull()
    // These are the plural categories used by the shipped locales (en/ja/zh/yue).
    val plural = if (Locale.forLanguageTag(locale).language == "en" && count == 1.0) "one" else "other"
    var text = (if (count != null) lookup("${key}_$plural") else null) ?: lookup(key) ?: key
    args.forEach { (name, replacement) ->
        text = text.replace("{{$name}}", replacement).replace("{$name}", replacement)
    }
    return text
}
