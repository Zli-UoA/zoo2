@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun

@JsFun(
    """(left, right, locale, numeric, base) => {
  const cache = globalThis.__zooCollators ??= new Map();
  const key = locale + ":" + numeric + ":" + base;
  let collator = cache.get(key);
  if (!collator) {
    collator = new Intl.Collator(locale, {numeric, ...(base ? {sensitivity: 'base'} : {})});
    cache.set(key, collator);
  }
  return collator.compare(left, right);
}"""
)
private external fun browserCompareLocale(
    left: String,
    right: String,
    locale: String,
    numeric: Boolean,
    base: Boolean,
): Int

actual fun compareLocaleText(
    left: String,
    right: String,
    locale: String,
    numeric: Boolean,
    baseSensitivity: Boolean,
): Int =
    browserCompareLocale(
        left,
        right,
        when (locale) {
            "ja" -> "ja-JP"
            "en" -> "en-US"
            "zh-Hant" -> "zh-TW"
            "zh-Hans" -> "zh-CN"
            "yue" -> "yue-Hant-HK"
            else -> locale
        },
        numeric,
        baseSensitivity,
    )
