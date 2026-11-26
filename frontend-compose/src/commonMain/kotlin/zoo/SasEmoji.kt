package zoo

/**
 * Matrix の SAS の絵文字の表（https://spec.matrix.org/latest/client-server-api/#sas-method-emoji。Web の
 * `frontend/src/lib/e2ee/sasEmoji.ts` と同じ）。番号（0〜63）は `SasView.emoji` の値。名前は翻訳の
 * `e2ee.sas.emoji.<番号>` にある（[sasEmojiLabelKey]）。
 *
 * 相手のクライアントと同じ表でなければ、同じ鍵でも違う絵文字が出る。並びを変えない。
 */
val SAS_EMOJI: List<String> = listOf(
    "🐶", "🐱", "🦁", "🐎", "🦄", "🐷", "🐘", "🐰",
    "🐼", "🐓", "🐧", "🐢", "🐟", "🐙", "🦋", "🌷",
    "🌳", "🌵", "🍄", "🌏", "🌙", "☁️", "🔥", "🍌",
    "🍎", "🍓", "🌽", "🍕", "🎂", "❤️", "😀", "🤖",
    "🎩", "👓", "🔧", "🎅", "👍", "☂️", "⌛", "⏰",
    "🎁", "💡", "📕", "✏️", "📎", "✂️", "🔒", "🔑",
    "🔨", "☎️", "🏁", "🚂", "🚲", "✈️", "🚀", "🏆",
    "⚽", "🎸", "🎺", "🔔", "⚓", "🎧", "📁", "📌",
)

/** 番号 `index`（0〜63）の絵文字の名前の翻訳キー。 */
fun sasEmojiLabelKey(index: Int): String = "e2ee.sas.emoji.$index"

/** 表示する 1 つの絵文字。`label` は翻訳した名前。 */
data class SasEmojiItem(val index: Int, val emoji: String, val label: String)

/**
 * `SasView.emoji` の番号（0〜63 が 7 つ）を、絵文字と翻訳した名前にする。`translate` は翻訳キーを訳す関数
 * （`Platform.translate` など）。範囲外の番号は捨てる。
 */
fun sasEmojiItems(indices: ByteArray, translate: (String) -> String): List<SasEmojiItem> =
    indices.map { it.toInt() and 0xff }
        .filter { it in SAS_EMOJI.indices }
        .map { SasEmojiItem(it, SAS_EMOJI[it], translate(sasEmojiLabelKey(it))) }
