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
    // English uses one/other; ja, zh, yue and ko use other.
    val plural = if (Locale.forLanguageTag(locale).language == "en" && count == 1.0) "one" else "other"
    var text = (if (count != null) lookup("${key}_$plural") else null) ?: lookup(key) ?: key
    args.forEach { (name, replacement) ->
        text = text.replace("{{$name}}", replacement).replace("{$name}", replacement)
    }
    return text
}

/**
 * ログイン前の画面、通知、更新の処理など、Session がない所の翻訳。アプリの表示言語（設定の `locale`）の辞書を
 * assets から読む。ログイン後の画面は Session の `t` を使う（同じ辞書と同じ言語）。
 */
internal class AppTranslator(context: android.content.Context) {
    private val context = context.applicationContext
    private val prefs = this.context.getSharedPreferences("preferences", 0)
    val locale: String get() = prefs.getString("locale", "ja")?.takeIf { it in appLocales } ?: "ja"

    fun t(key: String, vararg args: Pair<String, String>): String {
        val current = locale
        val dictionary = synchronized(dictionaries) {
            dictionaries.getOrPut(current) {
                runCatching {
                    context.assets.open("locales/$current.json").bufferedReader().use { zoo.json.parseToJsonElement(it.readText()) as Row }
                }.getOrElse { JsonObject(emptyMap()) }
            }
        }
        return translateDictionary(dictionary, current, key, args.toMap())
    }

    private companion object {
        val dictionaries = mutableMapOf<String, Row>()
        val appLocales = setOf("ja", "en", "ko-KP", "ko-KR", "yue", "zh-Hans", "zh-Hant")
    }
}

/** Context から翻訳する（Composable でない所、Toast・通知・Worker）。 */
internal fun android.content.Context.appText(key: String, vararg args: Pair<String, String>): String =
    AppTranslator(this).t(key, *args)
