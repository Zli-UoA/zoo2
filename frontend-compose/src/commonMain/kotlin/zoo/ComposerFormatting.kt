package zoo

import androidx.compose.ui.text.TextRange

internal enum class ComposerFormat(val icon: String, val title: String, val open: String, val close: String) {
    Bold("bold", "composer.bold", "<b>", "</b>"),
    Italic("italic", "composer.italic", "<i>", "</i>"),
    Strike("strikethrough", "composer.strike", "<s>", "</s>"),
    Code("code", "composer.code", "```\n", "\n```"),
}

internal data class ComposerEdit(val text: String, val selection: TextRange)

internal fun insertComposerText(text: String, selection: TextRange, insertion: String): ComposerEdit {
    val start = selection.min.coerceIn(0, text.length)
    val end = selection.max.coerceIn(start, text.length)
    return ComposerEdit(text.replaceRange(start, end, insertion), TextRange(start + insertion.length))
}

internal fun formatComposerText(text: String, selection: TextRange, format: ComposerFormat): ComposerEdit {
    val start = selection.min.coerceIn(0, text.length)
    val end = selection.max.coerceIn(start, text.length)
    // MFM uses fixed three-backtick fences. Do not corrupt an existing fenced block.
    if (format == ComposerFormat.Code && text.substring(start, end).contains("```"))
        return ComposerEdit(text, selection)
    val before = if (format == ComposerFormat.Code && start > 0 && text[start - 1] != '\n') "\n" else ""
    val after = if (format == ComposerFormat.Code && end < text.length && text[end] != '\n') "\n" else ""
    val opening = before + format.open
    return ComposerEdit(
        text.substring(0, start) + opening + text.substring(start, end) + format.close + after + text.substring(end),
        TextRange(start + opening.length, end + opening.length),
    )
}
