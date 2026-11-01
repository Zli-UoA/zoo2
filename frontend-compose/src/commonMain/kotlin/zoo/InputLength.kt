package zoo

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue

/** HTML-style UTF-16 input limits apply after composition, preserving the untouched suffix. */
internal fun limitFieldEdit(
    previous: TextFieldValue,
    next: TextFieldValue,
    maximum: Int?,
): TextFieldValue {
    require(maximum == null || maximum >= 0)
    if (
        maximum == null ||
            next.composition != null ||
            next.text.length <= maximum ||
            next.text.length <= previous.text.length
    )
        return next
    var prefix = 0
    while (
        prefix < previous.text.length &&
            prefix < next.text.length &&
            previous.text[prefix] == next.text[prefix]
    ) prefix++
    var suffix = 0
    while (
        suffix < previous.text.length - prefix &&
            suffix < next.text.length - prefix &&
            previous.text[previous.text.lastIndex - suffix] ==
                next.text[next.text.lastIndex - suffix]
    ) suffix++
    val inserted = next.text.substring(prefix, next.text.length - suffix)
    val removed = previous.text.length - prefix - suffix
    var acceptedLength = (maximum - previous.text.length + removed).coerceIn(0, inserted.length)
    if (
        acceptedLength > 0 &&
            acceptedLength < inserted.length &&
            inserted[acceptedLength - 1].isHighSurrogate() &&
            inserted[acceptedLength].isLowSurrogate()
    )
        acceptedLength--
    val text = next.text.take(prefix) + inserted.take(acceptedLength) + next.text.takeLast(suffix)
    if (text == previous.text) return previous.copy(composition = null)
    fun offset(value: Int): Int =
        when {
            value <= prefix -> value
            value < next.text.length - suffix ->
                prefix + (value - prefix).coerceAtMost(acceptedLength)
            else -> value - inserted.length + acceptedLength
        }.coerceIn(0, text.length)
    return TextFieldValue(text, TextRange(offset(next.selection.start), offset(next.selection.end)))
}
