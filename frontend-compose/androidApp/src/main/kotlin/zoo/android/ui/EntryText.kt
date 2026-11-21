package zoo.android.ui

import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.withStyle

private val inlineFormat = Regex("""(?m)\*\*([^*\n]+)\*\*|\*([^*\n]+)\*|`([^`\n]+)`|^> (.+)$""")

internal fun formatEntry(body: String) = buildAnnotatedString {
    var end = 0
    for (match in inlineFormat.findAll(body)) {
        append(body, end, match.range.first)
        when {
            match.groups[1] != null -> withStyle(SpanStyle(fontWeight = AndroidWeights.emphasis)) { append(match.groupValues[1]) }
            match.groups[2] != null -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(match.groupValues[2]) }
            match.groups[3] != null -> withStyle(SpanStyle(fontFamily = FontFamily.Monospace)) { append(match.groupValues[3]) }
            else -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append("│ "); append(match.groupValues[4]) }
        }
        end = match.range.last + 1
    }
    append(body, end, body.length)
}
