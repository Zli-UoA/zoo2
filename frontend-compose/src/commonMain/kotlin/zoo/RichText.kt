package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.*
import androidx.compose.ui.text.style.*
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import org.jetbrains.compose.resources.ExperimentalResourceApi
import zoo_compose.generated.resources.Res

/** Pure Kotlin inline scanner. Delimiters inside code remain literal. */
data class Inline(val value: String, val kind: String = "text", val url: String? = null)

fun parseInline(source: String): List<Inline> {
    val pattern =
        Regex(
            "(`[^`\\n]+`|<small>[\\s\\S]*?</small>|<plain>[\\s\\S]*?</plain>|\\*\\*[^*]+\\*\\*|~~[^~]+~~|<b>[\\s\\S]*?</b>|<i>[\\s\\S]*?</i>|\\*[^*\\n]+\\*|\\[[^]\\n]+]\\(https?://[^)\\s]+\\)|https?://[^\\s<>]+|@[A-Za-z0-9._-]+(?:@[A-Za-z0-9.-]+)?|#[^\\s#.,!?:;<>]+|:[a-zA-Z0-9_+-]+:)"
        )
    val out = mutableListOf<Inline>()
    var end = 0
    pattern.findAll(source).forEach { match ->
        if (match.range.first > end) out += Inline(source.substring(end, match.range.first))
        val s = match.value
        out +=
            when {
                s.startsWith('`') -> Inline(s.drop(1).dropLast(1), "code")
                s.startsWith("<plain>") -> Inline(s.drop(7).dropLast(8), "plain")
                s.startsWith("<small>") -> Inline(s.drop(7).dropLast(8), "small")
                s.startsWith("**") -> Inline(s.drop(2).dropLast(2), "bold")
                s.startsWith("<b>") -> Inline(s.drop(3).dropLast(4), "bold")
                s.startsWith("<i>") -> Inline(s.drop(3).dropLast(4), "italic")
                s.startsWith("~~") -> Inline(s.drop(2).dropLast(2), "strike")
                s.startsWith('*') -> Inline(s.drop(1).dropLast(1), "italic")
                s.startsWith('[') ->
                    Inline(
                        s.substringAfter('[').substringBefore(']'),
                        "link",
                        s.substringAfter("](").dropLast(1),
                    )
                s.startsWith("http") -> Inline(s, "link", s)
                s.startsWith('@') -> Inline(s, "mention")
                s.startsWith('#') -> Inline(s, "hashtag")
                s.startsWith(':') -> Inline(s, "emoji")
                else -> Inline(s)
            }
        end = match.range.last + 1
    }
    if (end < source.length) out += Inline(source.substring(end))
    return out
}

data class MessageHeadingBlock(val content: String, val level: Int? = null)

/** Heading markers only belong to root text, never to code or an MFM subtree. */
fun messageHeadingBlocks(content: String): List<MessageHeadingBlock> {
    val protectedRanges = BooleanArray(content.length)
    var cursor = 0
    richSegments(content).forEach { (text, embedded) ->
        if (embedded) for (index in cursor until cursor + text.length) protectedRanges[index] = true
        cursor += text.length
    }
    val blocks = mutableListOf<MessageHeadingBlock>()
    val pending = mutableListOf<String>()
    fun flush() {
        if (pending.isNotEmpty()) {
            val text = pending.joinToString("\n")
            if (text.isNotEmpty()) blocks += MessageHeadingBlock(text)
            pending.clear()
        }
    }
    var offset = 0
    content.lines().forEach { line ->
        val marker = Regex("^ {0,3}(#{1,6})[ \t]+").find(line)
        val source = line.substring(marker?.value?.length ?: 0)
        if (
            marker != null &&
                source.isNotBlank() &&
                protectedRanges.getOrNull(offset + marker.value.indexOf('#')) != true &&
                !Regex("^([^\n]+) (検索|Search)$").matches(line)
        ) {
            flush()
            blocks +=
                MessageHeadingBlock(
                    source.replace(Regex("[ \t]+#+[ \t]*$"), "").trimEnd(),
                    marker.groupValues[1].length,
                )
        } else pending += line
        offset += line.length + 1
    }
    flush()
    return blocks
}

@Composable
private fun RichSelection(enabled: Boolean, content: @Composable () -> Unit) {
    if (enabled) SelectionContainer { content() } else content()
}

@OptIn(ExperimentalResourceApi::class)
@Composable
fun RichText(
    content: String,
    users: List<Row> = emptyList(),
    depth: Int = 0,
    editedAt: String? = null,
    maxLines: Int = Int.MAX_VALUE,
    selection: Boolean = true,
) {
    if (depth > 12) {
        Text(content)
        return
    }
    val search = remember(content) { Regex("^([^\\n]+) (検索|Search)$").matchEntire(content) }
    if (search != null) {
        val app = LocalApp.current
        SearchQuery(app, search.groupValues[1])
        return
    }
    val headings =
        remember(content) { if (depth == 0) messageHeadingBlocks(content) else emptyList() }
    if (headings.any { it.level != null }) {
        val inherited = LocalMfmStyle.current
        val baseSize =
            inherited.fontSize.takeIf { it != TextUnit.Unspecified }?.value
                ?: MaterialTheme.typography.bodyMedium.fontSize.value
        val scales = listOf(1.75f, 1.5f, 1.25f, 1.125f, 1f, .9f)
        val density = androidx.compose.ui.platform.LocalDensity.current
        RichSelection(selection) {
            Column(Modifier.fillMaxWidth()) {
                headings.forEachIndexed { index, block ->
                    val previous = headings.getOrNull(index - 1)
                    val previousMargin =
                        previous?.level?.let { baseSize * scales[it - 1] * .25f } ?: 0f
                    val topMargin =
                        if (index == 0) 0f
                        else block.level?.let { baseSize * scales[it - 1] * .6f } ?: 0f
                    if (index > 0)
                        Spacer(
                            Modifier.height(
                                with(density) { maxOf(previousMargin, topMargin).sp.toDp() }
                            )
                        )
                    if (block.level != null) {
                        val size = baseSize * scales[block.level - 1]
                        Box(
                            Modifier.fillMaxWidth()
                                .then(
                                    webControlModifier(
                                        "heading",
                                        attributes = mapOf("aria-level" to block.level.toString()),
                                    )
                                )
                                .semantics { heading() }
                        ) {
                            CompositionLocalProvider(
                                LocalMfmStyle provides
                                    inherited.merge(
                                        TextStyle(
                                            fontSize = size.sp,
                                            fontWeight = FontWeight.Bold,
                                            lineHeight = (size * 1.35f).sp,
                                        )
                                    )
                            ) {
                                RichText(block.content, users, depth + 1, selection = false)
                            }
                        }
                    } else RichText(block.content, users, depth + 1, selection = false)
                }
            }
        }
        return
    }
    val segments = remember(content) { richSegments(content) }
    var shortcodes by remember { mutableStateOf<JsonObject>(JsonObject(emptyMap())) }
    LaunchedEffect(Unit) {
        shortcodes =
            json
                .parseToJsonElement(Res.readBytes("files/shortcodes.json").decodeToString())
                .jsonObject
    }
    RichSelection(selection) {
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            val inline = mutableListOf<Pair<String, Boolean>>()
            segments.forEach { segment ->
                val text = segment.first
                val block =
                    text.startsWith("```") ||
                        text.startsWith("\\[") ||
                        text.startsWith("<center>") ||
                        text.startsWith(">")
                if (block) {
                    if (inline.isNotEmpty()) {
                        InlineParagraph(inline.toList(), users, shortcodes, depth)
                        inline.clear()
                    }
                    when {
                        text.startsWith("```") ->
                            CodeBlock(
                                text.substringAfter('\n').substringBeforeLast("```"),
                                text.drop(3).substringBefore('\n').trim(),
                            )
                        text.startsWith("\\[") -> MathText(text.drop(2).dropLast(2))
                        text.startsWith("<center>") ->
                            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                                RichText(text.drop(8).dropLast(9), users, depth + 1)
                            }
                        else ->
                            Row {
                                Box(
                                    Modifier.width(3.dp)
                                        .height(20.dp)
                                        .background(MaterialTheme.colorScheme.outline)
                                )
                                Box(Modifier.padding(start = 12.dp)) {
                                    RichText(
                                        text.lines().joinToString("\n") {
                                            it.removePrefix(">").trimStart()
                                        },
                                        users,
                                        depth + 1,
                                    )
                                }
                            }
                    }
                } else inline += segment
            }
            if (inline.isNotEmpty())
                InlineParagraph(inline, users, shortcodes, depth, editedAt, maxLines)
        }
    }
}

@Composable
private fun InlineParagraph(
    segments: List<Pair<String, Boolean>>,
    users: List<Row>,
    shortcodes: Row,
    depth: Int,
    editedAt: String? = null,
    maxLines: Int = Int.MAX_VALUE,
) {
    val app = LocalApp.current
    val fonts = LocalFonts.current
    val channels by app.repo.channels.rows.collectAsState()
    val inheritedStyle = LocalMfmStyle.current
    val mentionForeground = MaterialTheme.colorScheme.onSecondaryContainer
    val mentionBackground = MaterialTheme.colorScheme.secondaryContainer
    val accent = LocalLinkColor.current
    val codeBackground = MaterialTheme.colorScheme.surfaceVariant
    val density = androidx.compose.ui.platform.LocalDensity.current
    val sizes = remember(segments) { mutableStateMapOf<String, androidx.compose.ui.unit.IntSize>() }
    val embedded =
        segments
            .mapIndexedNotNull { index, segment ->
                if (segment.second) index.toString() to segment.first else null
            }
            .toMap()
    val mentions = mutableMapOf<String, String>()
    val text = buildAnnotatedString {
        fun write(
            source: String,
            level: Int = 0,
            inheritedWeight: FontWeight? = inheritedStyle.fontWeight,
        ) {
            parseInline(source).forEach { token ->
                var value = token.value
                if (token.kind == "emoji")
                    value = (shortcodes[value.trim(':')] as? JsonPrimitive)?.contentOrNull ?: value
                val user = users.find { it.text("user_name") == value.removePrefix("@") }

                val inlineWeight = if (token.kind == "bold") FontWeight.Bold else inheritedWeight
                val style =
                    when (token.kind) {
                        "bold" -> SpanStyle(fontWeight = FontWeight.Bold)
                        "italic" -> SpanStyle(fontStyle = FontStyle.Italic)
                        "strike" -> SpanStyle(textDecoration = TextDecoration.LineThrough)
                        "small" -> SpanStyle(fontSize = 11.2.sp)
                        "code" -> SpanStyle(fontFamily = fonts.mono, background = codeBackground)
                        "mention" ->
                            SpanStyle(
                                color = mentionForeground,
                                background = mentionBackground,
                                fontWeight = FontWeight.Medium,
                            )
                        "hashtag",
                        "link" -> SpanStyle(color = accent)
                        else -> SpanStyle()
                    }
                val channel =
                    if (token.kind == "hashtag")
                        channels.find {
                            it.text("channel_name") == value.removePrefix("#") &&
                                !it.present("deleted_at")
                        }
                    else null
                val remote =
                    if (token.kind == "mention" && value.count { it == '@' } == 2)
                        value.drop(1).split('@').takeIf { it.size == 2 && it[1].contains('.') }
                    else null
                val link =
                    token.url
                        ?: channel?.let { "/channels/${it.text("id")}" }
                        ?: remote?.let { "https://${it[1]}/@${it[0]}" }
                if (token.kind == "mention" && link == null) {
                    val id = "mention-${mentions.size}"
                    mentions[id] = value
                    appendInlineContent(id, value)
                } else if (link != null)
                    withLink(
                        LinkAnnotation.Url(
                            link,
                            linkInteractionListener = {
                                if (link.startsWith('/')) app.navigate(link)
                                else app.platform.action("open:$link")
                            },
                        )
                    ) {
                        withStyle(style) {
                            append(
                                fontText(
                                    if (token.kind == "link" && token.value == token.url)
                                        value.replace(
                                            Regex("^https?://", RegexOption.IGNORE_CASE),
                                            "",
                                        )
                                    else value,
                                    fonts,
                                    inlineWeight,
                                )
                            )
                        }
                    }
                else
                    withStyle(style) {
                        if (token.kind in setOf("bold", "italic", "strike", "small") && level < 8)
                            write(value, level + 1, inlineWeight)
                        else if (token.kind == "code") append(value)
                        else append(fontText(value, fonts, inlineWeight))
                    }
            }
        }
        segments.forEachIndexed { index, segment ->
            if (segment.second) appendInlineContent(index.toString(), segment.first)
            else write(segment.first)
        }
        if (editedAt != null) {
            append(" ")
            appendInlineContent("edited", " ")
        }
    }
    val content =
        embedded.mapValues { (id, source) ->
            val measured = sizes[id]
            val width =
                measured?.let { with(density) { it.width.toSp() } }
                    ?: (source.length * 7).coerceIn(16, 600).sp
            val height = measured?.let { with(density) { it.height.toSp() } } ?: 20.sp
            androidx.compose.foundation.text.InlineTextContent(
                Placeholder(width, height, PlaceholderVerticalAlign.TextCenter)
            ) {
                Box(
                    Modifier.wrapContentSize(unbounded = true)
                        .then(Modifier.onSizeChanged { sizes[id] = it })
                ) {
                    if (source.startsWith("$["))
                        MfmEffect(source.drop(2).dropLast(1), users, depth + 1)
                    else if (source.startsWith("<plain>")) Text(source.drop(7).dropLast(8))
                    else MathInline(source.drop(2).dropLast(2))
                }
            }
        }
    val mentionContent =
        mentions.mapValues { (id, value) ->
            val measured = sizes[id]
            androidx.compose.foundation.text.InlineTextContent(
                Placeholder(
                    measured?.let { with(density) { it.width.toSp() } }
                        ?: (value.length * 7 + 4).sp,
                    measured?.let { with(density) { it.height.toSp() } } ?: 20.sp,
                    PlaceholderVerticalAlign.TextCenter,
                )
            ) {
                androidx.compose.material3.Text(
                    fontText(value, fonts, FontWeight.Medium),
                    Modifier.wrapContentSize(unbounded = true)
                        .onSizeChanged { sizes[id] = it }
                        .background(mentionBackground, RoundedCornerShape(4.dp))
                        .padding(horizontal = 2.dp),
                    color = mentionForeground,
                    style =
                        MaterialTheme.typography.bodyMedium
                            .merge(inheritedStyle)
                            .copy(fontFamily = fonts.latin, lineHeight = TextUnit.Unspecified),
                    fontWeight = FontWeight.Medium,
                )
            }
        }
    val paragraphLayout = remember { arrayOfNulls<TextLayoutResult>(1) }
    val currentText by rememberUpdatedState(text)
    InlineLinks(text) { displayText, linkLayout ->
        androidx.compose.material3.Text(
            displayText,
            modifier =
                Modifier.preventMessageHold { position ->
                    paragraphLayout[0]?.let {
                        val offset = it.getOffsetForPosition(position)
                        currentText.getLinkAnnotations(offset, offset).isNotEmpty()
                    } ?: false
                },
            onTextLayout = {
                paragraphLayout[0] = it
                linkLayout(it)
            },
            maxLines = maxLines,
            overflow = TextOverflow.Ellipsis,
            inlineContent =
                content +
                    mentionContent +
                    if (editedAt != null)
                        mapOf(
                            "edited" to
                                androidx.compose.foundation.text.InlineTextContent(
                                    Placeholder(12.sp, 12.sp, PlaceholderVerticalAlign.TextCenter)
                                ) {
                                    Hint(
                                        "${app.t("messages.edited")} · ${app.platform.fullTimestamp(editedAt)}"
                                    ) { hintModifier ->
                                        Icon(
                                            "pencil",
                                            app.t("messages.edited"),
                                            hintModifier.size(12.dp),
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                }
                        )
                    else emptyMap(),
            style = MaterialTheme.typography.bodyMedium.merge(inheritedStyle),
            color =
                inheritedStyle.color.takeIf { it != androidx.compose.ui.graphics.Color.Unspecified }
                    ?: MaterialTheme.colorScheme.onBackground,
        )
    }
}

fun cardColor(value: String): Color? {
    val raw =
        when (value) {
            "good" -> "2eb886"
            "warning" -> "daa038"
            "danger" -> "a30200"
            else -> value.removePrefix("#")
        }
    return raw.takeIf { Regex("[a-fA-F0-9]{6}").matches(it) }
        ?.toLongOrNull(16)
        ?.let { Color((0xff000000L or it).toInt()) }
}

@Composable
fun MessageCard(card: Row, attachment: Boolean = false) {
    val app = LocalApp.current
    val colors = MaterialTheme.colorScheme
    val accent = cardColor(card.text("color")) ?: colors.secondary
    val shape = RoundedCornerShape(if (attachment) 6.dp else 10.dp)
    val metadata =
        listOf("source", "context")
            .map { card.text(it) }
            .filter { it.isNotEmpty() }
            .joinToString(" · ")
    BoxWithConstraints(
        Modifier.widthIn(max = if (attachment) 672.dp else 576.dp)
            .fillMaxWidth()
            .padding(vertical = if (attachment) 8.dp else 4.dp)
            .then(
                if (attachment) Modifier
                else Modifier.background(colors.surface, shape).border(1.dp, colors.outline, shape)
            )
            .clip(shape)
    ) {
        val cardWidth = maxWidth - if (attachment) 28.dp else 29.dp
        Row(Modifier.height(IntrinsicSize.Min)) {
            Box(Modifier.width(4.dp).fillMaxHeight().background(accent))
            Column(
                Modifier.weight(1f)
                    .padding(
                        start = if (attachment) 16.dp else 12.dp,
                        end = if (attachment) 8.dp else 13.dp,
                        top = if (attachment) 4.dp else 13.dp,
                        bottom = if (attachment) 4.dp else 13.dp,
                    )
            ) {
                if (!attachment && (metadata.isNotEmpty() || card.present("badge")))
                    Row(
                        Modifier.fillMaxWidth().padding(bottom = 8.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon("external-link", modifier = Modifier.size(16.dp))
                        Text(metadata, Modifier.weight(1f), small = true, muted = true)
                        if (card.present("badge"))
                            Text(
                                card.text("badge"),
                                Modifier.border(1.dp, colors.outline, RoundedCornerShape(4.dp))
                                    .padding(horizontal = 7.dp, vertical = 3.dp),
                                small = true,
                                muted = true,
                            )
                    }
                val url = publicUrl(card.text("url"))
                Text(
                    card.text("title"),
                    if (url != null) Modifier.nativeNavigation(url.toString()) else Modifier,
                    weight = FontWeight.SemiBold,
                    color = if (url != null) LocalLinkColor.current else Color.Unspecified,
                )
                if (card.present("description"))
                    Column(Modifier.padding(top = 8.dp)) {
                        val description = card.text("description")
                        var expanded by remember(description) { mutableStateOf(false) }
                        var overflows by remember(description) { mutableStateOf(false) }
                        androidx.compose.material3.Text(
                            fontText(description, LocalFonts.current),
                            style = MaterialTheme.typography.bodyMedium,
                            color = colors.onSurfaceVariant,
                            maxLines = if (expanded) Int.MAX_VALUE else 8,
                            overflow = TextOverflow.Clip,
                            onTextLayout = { if (!expanded) overflows = it.hasVisualOverflow },
                        )
                        var toggleFocused by remember { mutableStateOf(false) }
                        if (overflows || expanded)
                            Row(
                                Modifier.padding(top = 4.dp)
                                    .then(webControlModifier("button", expanded = expanded))
                                    .onFocusChanged { toggleFocused = it.isFocused }
                                    .focusInputModality()
                                    .focusVisibleRing(
                                        8.dp,
                                        (if (colors.background == Color.Black) Color(0xffbdbdbd)
                                            else Color(0xff737373))
                                            .copy(alpha = .5f),
                                        toggleFocused && FocusInputModality.keyboard,
                                    )
                                    .height(28.dp)
                                    .border(
                                        1.dp,
                                        if (toggleFocused && FocusInputModality.keyboard)
                                            (if (colors.background == Color.Black) Color(0xffbdbdbd)
                                            else Color(0xff737373))
                                        else Color.Transparent,
                                        RoundedCornerShape(8.dp),
                                    )
                                    .padding(horizontal = 1.dp)
                                    .clickable(role = Role.Button) { expanded = !expanded }
                                    .semantics {
                                        contentDescription =
                                            app.t(
                                                if (expanded) "cards.showLess" else "cards.showMore"
                                            )
                                    },
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                Icon(
                                    if (expanded) "chevron-up" else "chevron-down",
                                    modifier = Modifier.size(16.dp),
                                    color = LocalLinkColor.current,
                                )
                                Text(
                                    app.t(if (expanded) "cards.showLess" else "cards.showMore"),
                                    size = 12.8.sp,
                                    weight = FontWeight.Medium,
                                    color = LocalLinkColor.current,
                                )
                            }
                    }
                val fields = card.array("fields").rows()
                if (fields.isNotEmpty())
                    Column(
                        Modifier.padding(top = 12.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        var index = 0
                        while (index < fields.size) {
                            val field = fields[index]
                            if (cardWidth >= 384.dp && field.flag("short")) {
                                Row(horizontalArrangement = Arrangement.spacedBy(24.dp)) {
                                    CardField(app, field, Modifier.weight(1f))
                                    if (fields.getOrNull(index + 1)?.flag("short") == true) {
                                        CardField(app, fields[index + 1], Modifier.weight(1f))
                                        index++
                                    } else Spacer(Modifier.weight(1f))
                                }
                            } else CardField(app, field, Modifier.fillMaxWidth())
                            index++
                        }
                    }
                if (attachment && (metadata.isNotEmpty() || card.present("badge")))
                    Row(
                        Modifier.padding(top = 12.dp),
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon("external-link", modifier = Modifier.size(14.dp))
                        Text(
                            listOf(metadata, card.text("badge"))
                                .filter { it.isNotEmpty() }
                                .joinToString(" · "),
                            small = true,
                            muted = true,
                        )
                    }
                if (card.present("author"))
                    Text(
                        card.text("author"),
                        Modifier.padding(top = 8.dp),
                        small = true,
                        muted = true,
                    )
            }
        }
    }
}

@Composable
private fun CardField(app: Session, field: Row, modifier: Modifier) {
    val url = publicUrl(field.text("url"))?.takeIf { it.protocol.name == "https" }
    Column(modifier) {
        Text(field.text("title"), Modifier.padding(bottom = 4.dp), weight = FontWeight.SemiBold)
        val valueModifier =
            (if (url != null) Modifier.nativeNavigation(url.toString()) else Modifier)
        if (field.flag("code")) {
            val fonts = LocalFonts.current
            androidx.compose.material3.Text(
                fontText(field.text("value"), fonts.copy(latin = fonts.mono)),
                valueModifier
                    .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(4.dp))
                    .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(4.dp))
                    .padding(horizontal = 4.dp, vertical = 2.dp),
                style =
                    MaterialTheme.typography.bodySmall.copy(
                        fontFamily = fonts.mono,
                        fontSize = 12.sp,
                        lineHeight = 16.sp,
                    ),
                color =
                    if (url != null) LocalLinkColor.current
                    else MaterialTheme.colorScheme.onBackground,
            )
        } else
            Text(
                field.text("value"),
                valueModifier,
                color = if (url != null) LocalLinkColor.current else Color.Unspecified,
            )
    }
}

@Composable
fun EmojiPicker(
    app: Session,
    close: () -> Unit,
    initialSearch: String = "",
    initialKitchen: Boolean = false,
    initialRecents: List<String>? = null,
    kitchenCatalogUrl: String? = null,
    embedded: Boolean = false,
    select: (String) -> Unit,
) {
    var query by remember(initialSearch) { mutableStateOf(initialSearch) }
    var emojis by remember(app.env.locale) { mutableStateOf<List<Row>>(emptyList()) }
    var recents by remember { mutableStateOf<List<String>>(emptyList()) }
    var groups by remember(app.env.locale) { mutableStateOf<List<Row>>(emptyList()) }
    var loading by remember(app.env.locale) { mutableStateOf(true) }
    var tone by remember { mutableStateOf("") }
    var kitchen by remember(initialKitchen) { mutableStateOf(initialKitchen) }
    LaunchedEffect(app.env.locale) {
        try {
            val locale =
                when (app.env.locale) {
                    "ja" -> "ja"
                    "zh-Hant",
                    "zh-Hans",
                    "yue" -> "zh-hant"
                    else -> "en"
                }
            emojis =
                app.asset("/emojibase-data/$locale/data.json")
                    .rows()
                    .filter {
                        it.present("group") &&
                            (it["version"]?.jsonPrimitive?.doubleOrNull ?: 0.0) <= 16.0
                    }
                    .sortedBy { it.text("order").toIntOrNull() ?: Int.MAX_VALUE }
            groups =
                app.asset("/emojibase-data/$locale/messages.json").jsonObject.array("groups").rows()
            val raw = app.platform.invoke("storageRead", "zoo-recent-emoji")
            recents =
                initialRecents
                    ?: try {
                        json
                            .parseToJsonElement(raw)
                            .jsonArray
                            .mapNotNull {
                                (it as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull
                            }
                            .filter { it.isNotEmpty() }
                            .distinct()
                            .take(16)
                    } catch (_: Exception) {
                        emptyList()
                    }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {} finally {
            loading = false
        }
    }
    fun choose(value: String) {
        recents = (listOf(value) + recents).distinct().take(16)
        app.run {
            app.platform.invoke(
                "storageWrite",
                obj(
                        "key" to "zoo-recent-emoji",
                        "value" to JsonArray(recents.map(::JsonPrimitive)).toString(),
                    )
                    .toString(),
            )
        }
        select(value)
        close()
    }
    @Composable
    fun EmojiButton(value: String, label: String = value, modifier: Modifier = Modifier) {
        Box(
            modifier
                .height(36.dp)
                .clickable(role = androidx.compose.ui.semantics.Role.Button) { choose(value) }
                .semantics { contentDescription = label },
            contentAlignment = Alignment.Center,
        ) {
            Text(value, size = 20.sp)
        }
    }
    val content: @Composable ColumnScope.() -> Unit = pickerContent@{
        Row(
            Modifier.fillMaxWidth()
                .background(
                    MaterialTheme.colorScheme.surfaceVariant,
                    androidx.compose.foundation.shape.RoundedCornerShape(8.dp),
                )
                .padding(4.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            listOf(false to "reactions.tabEmoji", true to "reactions.tabKitchen").forEach {
                (tab, key) ->
                Button(
                    app.t(key),
                    modifier =
                        Modifier.weight(1f)
                            .then(webControlModifier("tab", selected = kitchen == tab))
                            .background(
                                if (kitchen == tab) MaterialTheme.colorScheme.background
                                else androidx.compose.ui.graphics.Color.Transparent,
                                androidx.compose.foundation.shape.RoundedCornerShape(6.dp),
                            ),
                ) {
                    kitchen = tab
                }
            }
        }
        if (kitchen) {
            KitchenPicker(app, ::choose, kitchenCatalogUrl)
            return@pickerContent
        }
        if (recents.isNotEmpty()) {
            androidx.compose.foundation.layout.FlowRow(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                recents.forEach { EmojiButton(it, modifier = Modifier.width(32.dp)) }
            }
            HorizontalDivider()
        }
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Bottom,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Field(query, app.t("reactions.searchEmoji"), Modifier.weight(1f), showLabel = false) {
                query = it
            }
            Menu(
                app.t("reactions.skinTone"),
                listOf("", "🏻", "🏼", "🏽", "🏾", "🏿").map { skin ->
                    MenuAction("", "👍$skin") { tone = skin }
                },
                icon = "chevron-down",
            )
        }
        val filtered =
            emojis.filter {
                query.isBlank() ||
                    (it.text("label") + it.text("emoji") + it["tags"].toString()).contains(
                        query.trim(),
                        true,
                    )
            }
        LazyColumn(
            Modifier.fillMaxWidth()
                .then(
                    webControlModifier(
                        "grid",
                        attributes = mapOf("aria-label" to app.t("reactions.tabEmoji")),
                    )
                )
                .height(320.dp)
                .border(
                    1.dp,
                    MaterialTheme.colorScheme.outline,
                    androidx.compose.foundation.shape.RoundedCornerShape(8.dp),
                )
        ) {
            if (loading)
                item {
                    Text(
                        app.t("reactions.emojiLoading"),
                        Modifier.fillMaxWidth().padding(12.dp),
                        small = true,
                        muted = true,
                    )
                }
            else if (filtered.isEmpty())
                item {
                    Text(
                        app.t("reactions.noSearchResults"),
                        Modifier.fillMaxWidth().padding(12.dp),
                        small = true,
                        muted = true,
                    )
                }
            filtered
                .groupBy { it.text("group") }
                .forEach { (group, rows) ->
                    item("group:$group") {
                        Text(
                            groups.find { it.text("order") == group }?.text("message").orEmpty(),
                            Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp),
                            small = true,
                            muted = true,
                            weight = FontWeight.Medium,
                        )
                    }
                    itemsIndexed(rows.chunked(8)) { rowIndex, row ->
                        Row(
                            Modifier.fillMaxWidth()
                                .then(
                                    webControlModifier(
                                        "row",
                                        attributes =
                                            mapOf("aria-rowindex" to (rowIndex + 1).toString()),
                                    )
                                )
                                .padding(horizontal = 4.dp),
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            row.forEachIndexed { columnIndex, emoji ->
                                val value =
                                    if (tone.isEmpty()) emoji.text("emoji")
                                    else
                                        emoji["skins"]
                                            ?.rows()
                                            ?.find { it.text("emoji").contains(tone) }
                                            ?.text("emoji") ?: emoji.text("emoji")
                                Box(
                                    Modifier.weight(1f)
                                        .then(
                                            webControlModifier(
                                                "gridcell",
                                                attributes =
                                                    mapOf(
                                                        "aria-colindex" to
                                                            (columnIndex + 1).toString()
                                                    ),
                                            )
                                        )
                                ) {
                                    EmojiButton(value, emoji.text("label"), Modifier.fillMaxWidth())
                                }
                            }
                            repeat(8 - row.size) { Spacer(Modifier.weight(1f)) }
                        }
                    }
                }
        }
    }
    if (embedded) Column(Modifier.fillMaxWidth(), content = content)
    else
        Panel(
            app.t("reactions.add"),
            close,
            448.dp,
            description =
                app.t(
                    if (kitchen) "reactions.kitchenDescription" else "reactions.emojiDescription"
                ),
            content = content,
        )
}

/** Find balanced MFM functions without swallowing nested closing brackets. */
fun richSegments(source: String): List<Pair<String, Boolean>> {
    val result = mutableListOf<Pair<String, Boolean>>()
    var at = 0
    var plain = 0
    while (at < source.length) {
        if (source[at] == '`' && !source.startsWith("```", at)) {
            val end = source.indexOf('`', at + 1)
            val newline = source.indexOf('\n', at + 1).let { if (it < 0) source.length else it }
            if (end >= 0 && end < newline) {
                at = end + 1
                continue
            }
        }
        if ((at == 0 || source[at - 1] == '\n') && source[at] == '>') {
            var end = source.indexOf('\n', at).let { if (it < 0) source.length else it }
            while (end < source.length && source.getOrNull(end + 1) == '>') end =
                source.indexOf('\n', end + 1).let { if (it < 0) source.length else it }
            if (at > plain) result += source.substring(plain, at) to false
            result += source.substring(at, end) to true
            at = end
            plain = end
            continue
        }
        val opener =
            when {
                source.startsWith("```", at) -> "```" to "```"
                source.startsWith("<plain>", at) -> "<plain>" to "</plain>"
                source.startsWith("\\[", at) -> "\\[" to "\\]"
                source.startsWith("\\(", at) -> "\\(" to "\\)"
                source.startsWith("<center>", at) -> "<center>" to "</center>"
                source.startsWith("$[", at) -> "$[" to "]"
                else -> null
            }
        if (opener == null) {
            at++
            continue
        }
        var end = -1
        if (opener.first == "$[") {
            var depth = 1
            var cursor = at + 2
            while (cursor < source.length && depth > 0) {
                if (source.startsWith("$[", cursor)) {
                    depth++
                    cursor += 2
                } else {
                    if (source[cursor] == ']') depth--
                    cursor++
                }
            }
            if (depth == 0) end = cursor
        } else
            source
                .indexOf(opener.second, at + opener.first.length)
                .takeIf { it >= 0 }
                ?.let { end = it + opener.second.length }
        if (end < 0) {
            at += opener.first.length
            continue
        }
        if (at > plain) result += source.substring(plain, at) to false
        result += source.substring(at, end) to true
        at = end
        plain = end
    }
    if (plain < source.length) result += source.substring(plain) to false
    return result
}

/** Mirror the server-owned integration card envelope; invalid data keeps the original text. */
fun messageCards(value: JsonElement?): List<Row>? =
    try {
        val parsed = if (value is JsonPrimitive) json.parseToJsonElement(value.content) else value
        val envelope = parsed as? JsonObject
        val cards =
            if (envelope?.containsKey("cards") == true)
                envelope["cards"]?.jsonArray?.map { it.jsonObject }
            else envelope?.let { listOf(it) }
        cards?.takeIf { list ->
            list.size in 1..101 &&
                list.all { card ->
                    (card["title"] as? JsonPrimitive)?.isString == true &&
                        card.text("title").isNotBlank() &&
                        card.text("title").length <= 2000 &&
                        listOf("description", "url", "source", "context", "author", "badge").all {
                            key ->
                            card[key] == null || (card[key] as? JsonPrimitive)?.isString == true
                        } &&
                        (card["fields"] == null ||
                            card["fields"] is JsonArray &&
                                card.array("fields").size <= 10 &&
                                card.array("fields").all { field ->
                                    field is JsonObject &&
                                        (field["title"] as? JsonPrimitive)?.isString == true &&
                                        field.text("title").length <= 100 &&
                                        (field["value"] as? JsonPrimitive)?.isString == true &&
                                        field.text("value").length <= 4000 &&
                                        (field["url"] == null ||
                                            (field["url"] as? JsonPrimitive)?.isString == true) &&
                                        listOf("short", "code").all { key ->
                                            field[key] == null ||
                                                (field[key] as? JsonPrimitive)?.let {
                                                    !it.isString && it.booleanOrNull != null
                                                } == true
                                        }
                                })
                }
        }
    } catch (_: Exception) {
        null
    }
