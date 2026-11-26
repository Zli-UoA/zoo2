package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.platform.LocalDensity
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
import zoo.highlightSegments
import zoo.publicUrlString
import zoo.richSegments
import zoo.previewUrls
import zoo.fetchedLinks

private const val emojiTag = "zoo-entry-emoji"
private const val channelLinkIconId = "zoo-channel-link-message"
private const val channelLinkTag = "zoo-channel-link"
/** 1 つの本文で、短い表示にするアプリ内リンクの上限（取得する行き先の数を抑える）。 */
private const val maxResolvedLinks = 20
private val linked = SpanStyle(textDecoration = TextDecoration.Underline)
private val monospace = SpanStyle(fontFamily = FontFamily.Monospace)

/** Search terms to mark in the text parts of an entry; code, math, emoji and mentions are left as they are. */
internal class EntryHighlight(val terms: List<String>, val style: SpanStyle)

/** このアプリのチャンネル・メッセージへのリンク。`label` は行き先が分かったときの短い表示。 */
internal class EntryChannelLink(val label: String?, val messageIcon: Boolean, val open: () -> Unit)

/**
 * What [formatEntry] draws besides the MFM: custom emoji, search marks, whether links can be opened, and the
 * links into this app (`channelLink` returns `null` for other URLs; `bare` tells that the URL itself was written).
 */
private class EntryFormat(
    val emoji: Set<String>, val mark: EntryHighlight?, val links: Boolean,
    val channelLink: ((url: String, bare: Boolean) -> EntryChannelLink?)?,
) {
    /** リンクのラベル用。中の URL は、リンクにせず、短くせず、行き先も取らない。 */
    fun withoutLinks() = EntryFormat(emoji, mark, links = false, channelLink = null)
}

internal fun formatEntry(
    body: String, emoji: Set<String> = emptySet(), highlight: EntryHighlight? = null, links: Boolean = true,
    channelLink: ((url: String, bare: Boolean) -> EntryChannelLink?)? = null,
) = buildAnnotatedString {
    val format = EntryFormat(emoji, highlight?.takeIf { it.terms.isNotEmpty() }, links, channelLink)
    messageHeadingBlocks(body).forEachIndexed { index, block ->
        if (index > 0) append('\n')
        val level = block.level
        if (level == null) appendBlocks(block.content, format)
        else withStyle(SpanStyle(fontSize = (1.55f - (level - 1) * .09f).em,
            fontWeight = AndroidWeights.emphasis)) { appendMfm(block.content, format) }
    }
}

/** [text] with the parts that hit [highlight]'s terms in its style. */
internal fun AnnotatedString.Builder.appendHighlighted(text: String, highlight: EntryHighlight?) {
    if (highlight == null) {
        append(text)
        return
    }
    highlightSegments(text, highlight.terms).forEach { segment ->
        if (segment.match) withStyle(highlight.style) { append(segment.text) } else append(segment.text)
    }
}

private fun AnnotatedString.Builder.appendBlocks(body: String, format: EntryFormat, depth: Int = 0) {
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
        appendMfm(body, format, depth)
        return
    }
    segments.forEach { (source, block) ->
        when {
            block && source.startsWith("```") ->
                withStyle(monospace) { append(mfmCodeBlock(source).body) }
            block && source.startsWith("<center>") ->
                withStyle(ParagraphStyle(textAlign = TextAlign.Center)) {
                    appendBlocks(source.removePrefix("<center>").removeSuffix("</center>").trim(), format, depth + 1)
                }
            block && source.startsWith("> ") -> {
                withStyle(SpanStyle(fontStyle = FontStyle.Italic)) {
                    source.lines().forEachIndexed { index, line ->
                        if (index > 0) append('\n')
                        append("│ ")
                        appendMfm(line.removePrefix(">").trimStart(), format, depth + 1)
                    }
                }
            }
            else -> appendMfm(source, format, depth)
        }
    }
}

private fun AnnotatedString.Builder.appendMfm(body: String, format: EntryFormat, depth: Int = 0) {
    if (depth >= 16) {
        append(body)
        return
    }
    parseInline(body).forEach { token ->
        val value = token.value
        when (token.kind) {
            "bold" -> withStyle(SpanStyle(fontWeight = AndroidWeights.emphasis)) { appendMfm(value, format, depth + 1) }
            "italic" -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { appendMfm(value, format, depth + 1) }
            "strike" -> withStyle(SpanStyle(textDecoration = TextDecoration.LineThrough)) { appendMfm(value, format, depth + 1) }
            "small" -> withStyle(SpanStyle(fontSize = .8.em)) { appendMfm(value, format, depth + 1) }
            "code", "math" -> withStyle(monospace) { append(value) }
            "effect" -> appendEffect(value, format, depth)
            "link" -> {
                val url = token.url?.let(::publicUrlString)
                if (url == null) append(value)
                else if (!format.links) {
                    // Plain text with the link's formatting; a bare URL is not parsed again.
                    if (value == token.url) appendHighlighted(value, format.mark) else appendMfm(value, format, depth + 1)
                }
                else {
                    val bare = token.bare
                    val app = format.channelLink?.invoke(url, bare)
                    val annotation = if (app == null) LinkAnnotation.Url(url, TextLinkStyles(style = linked))
                    else LinkAnnotation.Clickable(channelLinkTag, TextLinkStyles(style = linked)) { app.open() }
                    withLink(annotation) {
                        // 短くするのは URL の文字そのものだけ。検索語を強調している間は書き換えない。
                        val short = app?.label?.takeIf { bare && format.mark == null }
                        if (short != null) {
                            if (app.messageIcon) {
                                appendInlineContent(channelLinkIconId, "\uD83D\uDCAC")
                                append(' ')
                            }
                            append(short)
                        } else {
                            // A label may contain formatting. Strip its link annotations to avoid nested links.
                            val label = buildAnnotatedString {
                                if (value == token.url) appendHighlighted(value, format.mark)
                                else appendMfm(value, format.withoutLinks(), depth + 1)
                            }
                            append(AnnotatedString(label.text, label.spanStyles))
                        }
                    }
                }
            }
            "emoji" -> {
                if (value in format.emoji) {
                    pushStringAnnotation(emojiTag, value)
                    appendInlineContent(value, value)
                    pop()
                } else append(value)
            }
            "mention", "hashtag" -> withStyle(SpanStyle(fontWeight = AndroidWeights.emphasis)) { append(value) }
            else -> appendHighlighted(value, format.mark)
        }
    }
}

private fun AnnotatedString.Builder.appendEffect(source: String, format: EntryFormat, depth: Int) {
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
            appendMfm(ruby.base, format, depth + 1)
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
    withStyle(style) { appendMfm(effect.body, format, depth + 1) }
}

internal fun entryCode(value: String) = buildAnnotatedString { withStyle(monospace) { append(value) } }

internal fun entryLabel(value: String, url: String?) = buildAnnotatedString {
    if (url == null) append(value)
    else withLink(LinkAnnotation.Url(url, TextLinkStyles(style = linked))) { append(value) }
}

/**
 * [body] のうち、このアプリへのリンクと、その行き先の短い表示。行き先は、URL の文字そのものを書いたリンクだけ
 * 取得する。アプリ内のリンクを扱わない画面や、リンクがない本文では `null`。
 */
@Composable
private fun entryChannelLinks(body: String, links: Boolean): Map<String, EntryChannelLink>? {
    val provider = LocalChannelLinks.current
    if (provider == null || !links) return null
    val found = remember(body, provider) {
        val urls = linkedMapOf<String, Boolean>()
        formatEntry(body, channelLink = { url, bare ->
            if (provider.target(url) != null) urls[url] = bare || urls[url] == true
            null
        })
        urls.toList()
    }
    if (found.isEmpty()) return null
    // 取得するリンク先は本文ごとに上限がある。超えた分は URL のまま出す。
    val fetched = remember(found, provider) {
        fetchedLinks(
            previewUrls(body).mapNotNull { provider.target(it) },
            found.filter { it.second }.mapNotNull { provider.target(it.first) },
        )
    }
    var resolved = 0
    val result = found.associate { (url, bare) ->
        url to key(url) {
            val target = checkNotNull(provider.target(url))
            val allowed = target.linkedMessageId?.let { (target.channelId to it) in fetched } ?: true
            val view = if (bare && allowed && resolved++ < maxResolvedLinks) provider.view(target) else null
            val parent = (view as? ChannelLinkView.Message)?.parentId
            val label = when (view) {
                is ChannelLinkView.Message -> view.inlineLabel
                is ChannelLinkView.Channel -> view.channel
                else -> null
            }
            ChannelLinkState(label, view is ChannelLinkView.Message, target, parent)
        }
    }
    return remember(result, provider) {
        result.mapValues { (_, link) ->
            EntryChannelLink(link.label, link.messageIcon) { provider.open(link.target, link.parentId) }
        }
    }
}

private data class ChannelLinkState(
    val label: String?, val messageIcon: Boolean, val target: zoo.ChannelLinkTarget, val parentId: String?,
)

@Composable
internal fun EntryBody(
    body: String, color: Color, fontSize: TextUnit, emoji: Map<String, TimelineMedia>, maxLines: Int = Int.MAX_VALUE,
    highlight: EntryHighlight? = null, lineHeight: TextUnit = TextUnit.Unspecified, links: Boolean = true,
) {
    val loader = LocalTimelineImage.current
    val channelLinks = entryChannelLinks(body, links)
    val formatted = remember(body, emoji, highlight, links, channelLinks) {
        val lookup: ((String, Boolean) -> EntryChannelLink?)? =
            channelLinks?.let { found -> { url: String, _: Boolean -> found[url] } }
        formatEntry(body, emoji.keys, highlight, links, lookup)
    }
    val images = remember(formatted, emoji, loader) { mutableStateMapOf<String, ImageBitmap>() }
    LaunchedEffect(formatted, emoji, loader) {
        formatted.getStringAnnotations(emojiTag, 0, formatted.length).map { it.item }.distinct().take(32).forEach { code ->
            launch {
                val media = emoji[code] ?: return@launch
                loader(media)?.let { images[code] = it }
            }
        }
    }
    val iconSize = with(LocalDensity.current) { (if (fontSize == TextUnit.Unspecified) entryTextSize else fontSize).toDp() }
    val inline = images.toMap().mapValues { (code, image) ->
        InlineTextContent(Placeholder(1.3.em, 1.3.em, PlaceholderVerticalAlign.TextCenter)) {
            Image(image, contentDescription = code)
        }
    } + (channelLinkIconId to InlineTextContent(Placeholder(1.em, 1.em, PlaceholderVerticalAlign.TextCenter)) {
        UiGlyph(UiIcon.Bubble, color, iconSize)
    })
    Text(formatted, color = color, fontSize = fontSize, lineHeight = lineHeight, inlineContent = inline, maxLines = maxLines,
        overflow = TextOverflow.Ellipsis)
}
