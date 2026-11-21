package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.text.ParagraphStyle
import androidx.compose.ui.text.Placeholder
import androidx.compose.ui.text.PlaceholderVerticalAlign
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.em
import kotlinx.coroutines.launch
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import zoo.messageHeadingBlocks
import zoo.mfmCodeBlock
import zoo.mfmFont
import zoo.mfmFunction
import zoo.mfmRuby
import zoo.mfmUnixSeconds
import zoo.parseInline
import zoo.publicUrlString
import zoo.richSegments

private const val emojiTag = "zoo-entry-emoji"
private val linked = SpanStyle(textDecoration = TextDecoration.Underline)
private val monospace = SpanStyle(fontFamily = FontFamily.Monospace)

internal fun formatEntry(body: String, emoji: Set<String> = emptySet()) = buildAnnotatedString {
    messageHeadingBlocks(body).forEachIndexed { index, block ->
        if (index > 0) append('\n')
        val level = block.level
        if (level == null) appendBlocks(block.content, emoji)
        else withStyle(SpanStyle(fontSize = (1.55f - (level - 1) * .09f).em,
            fontWeight = AndroidWeights.emphasis)) { appendMfm(block.content, emoji) }
    }
}

private fun AnnotatedString.Builder.appendBlocks(body: String, emoji: Set<String>, depth: Int = 0) {
    if (depth >= 16) {
        append(body)
        return
    }
    // The scanner distinguishes actual block tags from escaped tags and literal regions.
    val segments = richSegments(body)
    if (segments.none { (source, block) ->
            block && (source.startsWith("```") || source.startsWith("<center>") || source.startsWith("> "))
        }
    ) {
        appendMfm(body, emoji, depth)
        return
    }
    segments.forEach { (source, block) ->
        when {
            block && source.startsWith("```") ->
                withStyle(monospace) { append(mfmCodeBlock(source).body) }
            block && source.startsWith("<center>") ->
                withStyle(ParagraphStyle(textAlign = TextAlign.Center)) {
                    appendBlocks(source.removePrefix("<center>").removeSuffix("</center>").trim(), emoji, depth + 1)
                }
            block && source.startsWith("> ") -> {
                withStyle(SpanStyle(fontStyle = FontStyle.Italic)) {
                    source.lines().forEachIndexed { index, line ->
                        if (index > 0) append('\n')
                        append("│ ")
                        appendMfm(line.removePrefix(">").trimStart(), emoji, depth + 1)
                    }
                }
            }
            else -> appendMfm(source, emoji, depth)
        }
    }
}

private fun AnnotatedString.Builder.appendMfm(body: String, emoji: Set<String>, depth: Int = 0) {
    if (depth >= 16) {
        append(body)
        return
    }
    parseInline(body).forEach { token ->
        val value = token.value
        when (token.kind) {
            "bold" -> withStyle(SpanStyle(fontWeight = AndroidWeights.emphasis)) { appendMfm(value, emoji, depth + 1) }
            "italic" -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { appendMfm(value, emoji, depth + 1) }
            "strike" -> withStyle(SpanStyle(textDecoration = TextDecoration.LineThrough)) { appendMfm(value, emoji, depth + 1) }
            "small" -> withStyle(SpanStyle(fontSize = .8.em)) { appendMfm(value, emoji, depth + 1) }
            "code", "math" -> withStyle(monospace) { append(value) }
            "effect" -> appendEffect(value, emoji, depth)
            "link" -> {
                val url = token.url?.let(::publicUrlString)
                if (url == null) append(value)
                else withLink(LinkAnnotation.Url(url, TextLinkStyles(style = linked))) {
                    // A label may contain formatting. Strip its link annotations to avoid nested links.
                    val label = buildAnnotatedString {
                        if (value == token.url) append(value) else appendMfm(value, emoji, depth + 1)
                    }
                    append(AnnotatedString(label.text, label.spanStyles))
                }
            }
            "emoji" -> {
                if (value in emoji) {
                    pushStringAnnotation(emojiTag, value)
                    appendInlineContent(value, value)
                    pop()
                } else append(value)
            }
            "mention", "hashtag" -> withStyle(SpanStyle(fontWeight = AndroidWeights.emphasis)) { append(value) }
            else -> append(value)
        }
    }
}

private fun AnnotatedString.Builder.appendEffect(source: String, emoji: Set<String>, depth: Int) {
    val effect = mfmFunction(source.substring(2, source.length - 1))
    val style = when (effect.name) {
        "fg" -> SpanStyle(color = effect.argb("color")?.let(::Color) ?: Color.Unspecified)
        "bg" -> SpanStyle(background = effect.argb("color")?.let(::Color) ?: Color.Unspecified)
        "x2", "x3", "x4", "scale" -> SpanStyle(fontSize = effect.scale(1f).y.coerceIn(.5f, 4f).em)
        "font" -> SpanStyle(fontFamily = when (mfmFont(effect.args)) {
            "serif", "math" -> FontFamily.Serif
            "monospace" -> FontFamily.Monospace
            "cursive", "fantasy" -> FontFamily.Cursive
            else -> null
        })
        "ruby" -> {
            val ruby = mfmRuby(effect.body)
            appendMfm(ruby.base, emoji, depth + 1)
            if (ruby.reading.isNotBlank()) withStyle(SpanStyle(fontSize = .7.em)) { append("（${ruby.reading}）") }
            return
        }
        "unixtime" -> {
            val seconds = mfmUnixSeconds(effect.body)
            if (seconds == null) append(effect.body)
            else append(DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM)
                .withZone(ZoneId.systemDefault()).format(Instant.ofEpochSecond(seconds)))
            return
        }
        else -> SpanStyle()
    }
    withStyle(style) { appendMfm(effect.body, emoji, depth + 1) }
}

internal fun entryCode(value: String) = buildAnnotatedString { withStyle(monospace) { append(value) } }

internal fun entryLabel(value: String, url: String?) = buildAnnotatedString {
    if (url == null) append(value)
    else withLink(LinkAnnotation.Url(url, TextLinkStyles(style = linked))) { append(value) }
}

@Composable
internal fun EntryBody(
    body: String, color: Color, fontSize: TextUnit, emoji: Map<String, TimelineMedia>, maxLines: Int = Int.MAX_VALUE,
) {
    val loader = LocalTimelineImage.current
    val formatted = remember(body, emoji) { formatEntry(body, emoji.keys) }
    val images = remember(formatted, emoji, loader) { mutableStateMapOf<String, ImageBitmap>() }
    LaunchedEffect(formatted, emoji, loader) {
        formatted.getStringAnnotations(emojiTag, 0, formatted.length).map { it.item }.distinct().take(32).forEach { code ->
            launch {
                val media = emoji[code] ?: return@launch
                loader(media)?.let { images[code] = it }
            }
        }
    }
    val inline = images.toMap().mapValues { (code, image) ->
        InlineTextContent(Placeholder(1.3.em, 1.3.em, PlaceholderVerticalAlign.TextCenter)) {
            Image(image, contentDescription = code)
        }
    }
    Text(formatted, color = color, fontSize = fontSize, inlineContent = inline, maxLines = maxLines, overflow = TextOverflow.Ellipsis)
}
