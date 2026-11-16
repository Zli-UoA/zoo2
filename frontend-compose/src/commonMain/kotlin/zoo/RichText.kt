package zoo

import androidx.compose.foundation.*
import kotlin.math.roundToInt
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.SubcomposeLayout
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.*
import androidx.compose.ui.text.style.*
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*
import org.jetbrains.compose.resources.ExperimentalResourceApi
import zoo_compose.generated.resources.Res

internal val LocalMfmInline = staticCompositionLocalOf { false }

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
    if (depth > 20) {
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
        remember(content, depth) { if (depth == 0) messageHeadingBlocks(content) else emptyList() }
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
    val app = LocalApp.current
    val segments = remember(content) { richSegments(content) }
    var fallbackShortcodes by remember { mutableStateOf<JsonObject>(JsonObject(emptyMap())) }
    LaunchedEffect(app) {
        fallbackShortcodes = loadShortcodes()
        while (true) {
            try { app.loadShortcodes() } catch (e: CancellationException) { throw e } catch (_: Exception) {}
            delay(3_600_000)
        }
    }
    val shortcodes = app.standardShortcodes ?: fallbackShortcodes
    RichSelection(selection) {
        Column(
            if (LocalMfmInline.current) Modifier else Modifier.fillMaxWidth().clipToBounds(),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            val inline = mutableListOf<Pair<String, Boolean>>()
            segments.forEach { segment ->
                val text = segment.first
                val block =
                    segment.second &&
                        (text.startsWith("```") ||
                            text.startsWith("\\[") ||
                            text.startsWith("<center>") ||
                            text.startsWith(">"))
                if (block) {
                    if (inline.isNotEmpty()) {
                        InlineParagraph(inline.toList(), users, shortcodes, depth)
                        inline.clear()
                    }
                    when {
                        text.startsWith("```") ->
                            mfmCodeBlock(text).let { CodeBlock(it.body, it.language) }
                        text.startsWith("\\[") -> MathText(text.drop(2).dropLast(2))
                        text.startsWith("<center>") ->
                            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                                CompositionLocalProvider(
                                    LocalMfmStyle provides LocalMfmStyle.current.copy(textAlign = TextAlign.Center),
                                    LocalMfmInline provides true,
                                ) {
                                    RichText(text.drop(8).dropLast(9), users, depth + 1)
                                }
                            }
                        else -> {
                            val quoteColor = MaterialTheme.colorScheme.outline
                            Box(Modifier.drawBehind {
                                drawRect(quoteColor, size = androidx.compose.ui.geometry.Size(3.dp.toPx(), size.height))
                            }.padding(start = 15.dp)) {
                                RichText(
                                    text.lines().joinToString("\n") { it.removePrefix(">").trimStart() },
                                    users,
                                    depth + 1,
                                )
                            }
                        }
                    }
                } else inline += segment
            }
            if (inline.isNotEmpty() || editedAt != null)
                InlineParagraph(inline, users, shortcodes, depth, editedAt, maxLines)
        }
    }
}

private val shortcodeLock = Mutex()
private var loadedShortcodes: Row? = null

/** Immutable bundled data is parsed once, rather than for every recycled message. */
@OptIn(ExperimentalResourceApi::class)
private suspend fun loadShortcodes(): Row = shortcodeLock.withLock {
    loadedShortcodes ?: json.parseToJsonElement(
        Res.readBytes("files/shortcodes.json").decodeToString()
    ).jsonObject.also { loadedShortcodes = it }
}

private data class EmbeddedMfm(val source: String, val style: SpanStyle, val inLink: Boolean)

private data class CachedParagraph(
    val text: AnnotatedString,
    val mentions: Map<String, String>,
    val emojiParts: Map<String, Row>,
    val accountParts: Map<String, Row>,
    val embeddedParts: Map<String, EmbeddedMfm>,
)

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
    val authzAccounts = app.accountProfiles.map { JsonObject(it + ("user_name" to JsonPrimitive(it.text("username")))) }
    val channels by app.repo.channels.rows.collectAsState()
    val inheritedStyle = MaterialTheme.typography.bodyMedium.merge(LocalMfmStyle.current)
    val inheritedInLink = LocalMfmInLink.current
    val mentionForeground = MaterialTheme.colorScheme.onSecondaryContainer
    val mentionBackground = MaterialTheme.colorScheme.secondaryContainer
    val accent = LocalLinkColor.current
    val codeBackground = MaterialTheme.colorScheme.surfaceVariant
    val density = androidx.compose.ui.platform.LocalDensity.current
    val joined = LocalJoinedEmojis.current
    val workspace = if (joined == null) workspaceEmojis(app) else emptyList()
    val registeredEmojis = remember(joined, workspace) {
        joined ?: workspace.flatMap { item ->
            val emoji = workspaceEmoji(item)
            listOf(emoji) + item.array("aliases").map { alias ->
                JsonObject(emoji + ("name" to JsonPrimitive(":${alias.jsonPrimitive.content}:")))
            }
        }
    }
    val activityPubEmojis = LocalActivityPubEmojis.current
    val remoteEmojis = remember(activityPubEmojis, registeredEmojis) { activityPubEmojis + registeredEmojis }
    val paragraph = remember(
        app, segments, users, authzAccounts, shortcodes, fonts, channels, inheritedStyle,
        mentionForeground, mentionBackground, accent, codeBackground, remoteEmojis, editedAt, inheritedInLink,
    ) {
        val mentions = mutableMapOf<String, String>()
        val emojiParts = mutableMapOf<String, Row>()
        val accountParts = mutableMapOf<String, Row>()
        val embeddedParts = mutableMapOf<String, EmbeddedMfm>()
        val text = buildAnnotatedString {
            fun write(
                source: String,
                level: Int = 0,
                inheritedWeight: FontWeight? = inheritedStyle.fontWeight,
                inheritedSpan: SpanStyle = inheritedStyle.toSpanStyle(),
                inLink: Boolean = inheritedInLink,
            ) {
                parseInline(source).forEach { token ->
                    var value = token.value
                    if (token.kind in setOf("effect", "math")) {
                        val id = "embedded-${embeddedParts.size}"
                        embeddedParts[id] = EmbeddedMfm(
                            if (token.kind == "math") "\\($value\\)" else value,
                            inheritedSpan,
                            inLink,
                        )
                        appendInlineContent(id, value)
                        return@forEach
                    }
                    if (token.kind == "accountEmoji") {
                        val account = authzAccounts.find { it.text("username") == value.removePrefix(":@").removeSuffix(":") }
                        if (account != null) {
                            val id = "account-emoji-${accountParts.size}"
                            accountParts[id] = account
                            appendInlineContent(id, value)
                            return@forEach
                        }
                    }
                    if (token.kind == "emoji") {
                        val remote = remoteEmojis.find { it.text("name") == value }
                        if (remote != null && (remote.flag("local") || Regex("^[a-f0-9]{64}$").matches(remote.text("image_id")))) {
                            val id = "apub-emoji-${emojiParts.size}"
                            emojiParts[id] = remote
                            appendInlineContent(id, value)
                            return@forEach
                        }
                        value = (shortcodes[value.trim(':')] as? JsonPrimitive)?.contentOrNull ?: value
                    }

                    val channel =
                        if (token.kind == "hashtag" && !inLink)
                            channels.find {
                                it.text("channel_name") == value.removePrefix("#") &&
                                    !it.present("deleted_at")
                            }
                        else null
                    val inlineWeight = if (token.kind == "bold") FontWeight.Bold else inheritedWeight
                    val style =
                        when (token.kind) {
                            "bold" -> SpanStyle(fontWeight = FontWeight.Bold)
                            "italic" -> SpanStyle(fontStyle = FontStyle.Italic)
                            "strike" -> SpanStyle(textDecoration = TextDecoration.LineThrough)
                            "small" -> SpanStyle(fontSize = (inheritedSpan.fontSize.value * .8f).sp)
                            "code" -> SpanStyle(fontFamily = fonts.mono, background = codeBackground)
                            "mention" -> if (inLink) SpanStyle() else
                                SpanStyle(
                                    color = mentionForeground,
                                    background = mentionBackground,
                                    fontWeight = FontWeight.Medium,
                                )
                            "hashtag" -> if (channel == null) SpanStyle() else SpanStyle(color = accent)
                            "link" -> SpanStyle(color = accent)
                            else -> SpanStyle()
                        }
                    val remote =
                        if (!inLink && token.kind == "mention" && value.count { it == '@' } == 2)
                            value.drop(1).split('@').takeIf { it.size == 2 && it[1].contains('.') }
                        else null
                    val link = if (inLink) null else
                        token.url
                            ?: channel?.let { "/channels/${it.text("id")}" }
                            ?: remote?.let { "https://${it[1]}/@${it[0]}" }
                    if (token.kind == "mention" && link == null && !inLink) {
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
                                if (token.kind == "link" && token.value != token.url && level + depth < 20)
                                    write(value, level + 1, inlineWeight, inheritedSpan.merge(style), true)
                                else append(
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
                            if ((token.kind in setOf("bold", "italic", "strike", "small") ||
                                    (token.kind == "link" && token.value != token.url)) && level + depth < 20)
                                write(value, level + 1, inlineWeight, inheritedSpan.merge(style), inLink)
                            else if (token.kind == "code") append(value)
                            else append(fontText(value, fonts, inlineWeight))
                        }
                }
            }
            write(segments.joinToString("") { it.first })
            if (editedAt != null) {
                append(" ")
                appendInlineContent("edited", " ")
            }
        }
        CachedParagraph(text, mentions.toMap(), emojiParts.toMap(), accountParts.toMap(), embeddedParts.toMap())
    }
    val text = paragraph.text
    val mentions = paragraph.mentions
    val emojiParts = paragraph.emojiParts
    val accountParts = paragraph.accountParts
    val embeddedContent = paragraph.embeddedParts.mapValues { (_, embedded) ->
        val source = embedded.source
        val ruby = source.startsWith("$[") && mfmFunction(source.drop(2).dropLast(1)).name == "ruby"
        MfmInlineBody(if (ruby) PlaceholderVerticalAlign.TextBottom else PlaceholderVerticalAlign.TextCenter) {
            CompositionLocalProvider(
                LocalMfmStyle provides inheritedStyle.merge(embedded.style),
                LocalMfmInline provides true,
                LocalMfmInLink provides embedded.inLink,
            ) {
                if (source.startsWith("$[")) MfmEffect(source.drop(2).dropLast(1), users, depth + 1)
                else MathInline(source.drop(2).dropLast(2))
            }
        }
    }
    val mentionContent = mentions.mapValues { (_, value) ->
        MfmInlineBody(PlaceholderVerticalAlign.TextCenter) {
            val account = authzAccounts.find { it.text("username") == value.removePrefix("@") }
            Row(
                Modifier.background(mentionBackground, RoundedCornerShape(4.dp))
                    .padding(horizontal = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                if (account != null) {
                    Text("@", color = mentionForeground)
                    Avatar(account, size = with(density) { inheritedStyle.fontSize.toDp() })
                }
                androidx.compose.material3.Text(
                    fontText(account?.let(::author) ?: value, fonts, FontWeight.Medium),
                    color = mentionForeground,
                    style = inheritedStyle.copy(fontFamily = fonts.latin, lineHeight = TextUnit.Unspecified),
                    fontWeight = FontWeight.Medium,
                )
            }
        }
    }
    val fixedContent = emojiParts.mapValues { (_, emoji) ->
        androidx.compose.foundation.text.InlineTextContent(
            Placeholder((1.5f * emojiRatio(emoji)).coerceAtMost(8f).em, 1.5.em, PlaceholderVerticalAlign.TextCenter)
        ) {
            ActivityPubEmoji(emoji, with(density) { (inheritedStyle.fontSize.value * 1.5f).sp.toDp() })
        }
    } + accountParts.mapValues { (_, account) ->
        androidx.compose.foundation.text.InlineTextContent(
            Placeholder(1.15.em, 1.15.em, PlaceholderVerticalAlign.TextCenter)
        ) {
            Avatar(account,
                modifier = Modifier.semantics { contentDescription = ":@${account.text("user_name")}:" },
                size = with(density) { (inheritedStyle.fontSize.value * 1.15f).sp.toDp() })
        }
    } + if (editedAt != null) mapOf(
        "edited" to androidx.compose.foundation.text.InlineTextContent(
            Placeholder(12.sp, 12.sp, PlaceholderVerticalAlign.TextCenter)
        ) {
            Hint("${app.t("messages.edited")} · ${app.platform.fullTimestamp(editedAt)}") { hintModifier ->
                Icon("pencil", app.t("messages.edited"), hintModifier.size(12.dp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    ) else emptyMap()
    val paragraphLayout = remember { arrayOfNulls<TextLayoutResult>(1) }
    val currentText by rememberUpdatedState(text)
    MeasuredMfmParagraph(text, embeddedContent + mentionContent, fixedContent) { inlineContent, reportLayout ->
        fun inSp(unit: TextUnit): Float =
            if (unit.isEm) unit.value * inheritedStyle.fontSize.value else unit.value
        val tallInline = inheritedStyle.lineHeight != TextUnit.Unspecified &&
            inlineContent.values.any { inSp(it.placeholder.height) > inSp(inheritedStyle.lineHeight) }
        // Android caches fixed/minimum line-height metrics from the first line. A later tall
        // replacement span must use natural metrics, otherwise its height is overwritten.
        val paragraphStyle = if (tallInline) inheritedStyle.copy(lineHeight = TextUnit.Unspecified)
            else inheritedStyle
        InlineLinks(text) { displayText, linkLayout ->
            androidx.compose.material3.Text(
                displayText,
                modifier = Modifier.preventMessageHold { position ->
                    paragraphLayout[0]?.let {
                        val offset = it.getOffsetForPosition(position)
                        currentText.getLinkAnnotations(offset, offset).isNotEmpty()
                    } ?: false
                },
                onTextLayout = {
                    paragraphLayout[0] = it
                    reportLayout(it)
                    linkLayout(it)
                },
                maxLines = maxLines,
                overflow = TextOverflow.Ellipsis,
                inlineContent = inlineContent,
                style = paragraphStyle,
                color = inheritedStyle.color.takeIf { it != Color.Unspecified }
                    ?: MaterialTheme.colorScheme.onBackground,
            )
        }
    }
}

private class MfmInlineBody(
    val alignment: PlaceholderVerticalAlign,
    val children: @Composable () -> Unit,
)

/** Measure each body once, then lay out the paragraph with its final placeholder dimensions. */
@Composable
private fun MeasuredMfmParagraph(
    text: AnnotatedString,
    bodies: Map<String, MfmInlineBody>,
    fixed: Map<String, androidx.compose.foundation.text.InlineTextContent>,
    content: @Composable (Map<String, androidx.compose.foundation.text.InlineTextContent>, (TextLayoutResult) -> Unit) -> Unit,
) {
    val density = androidx.compose.ui.platform.LocalDensity.current
    val textLayout = remember(text) { arrayOfNulls<TextLayoutResult>(1) }
    SubcomposeLayout { constraints ->
        val measured = bodies.mapValues { (id, body) ->
            subcompose("body-$id") { Box { body.children() } }.single()
                .measure(Constraints(maxWidth = constraints.maxWidth))
        }
        val placeholders = measured.mapValues { (id, child) ->
            androidx.compose.foundation.text.InlineTextContent(
                Placeholder(
                    with(density) { child.width.coerceAtLeast(1).toSp() },
                    with(density) { child.height.coerceAtLeast(1).toSp() },
                    bodies.getValue(id).alignment,
                )
            ) {}
        } + fixed
        val paragraph = subcompose("paragraph") {
            content(placeholders) { textLayout[0] = it }
        }.single().measure(constraints)
        val ids = text.getStringAnnotations(0, text.length)
            .filter { it.item in placeholders }.map { it.item }
        val positions = textLayout[0]?.placeholderRects.orEmpty()
        layout(paragraph.width, paragraph.height) {
            paragraph.place(0, 0)
            ids.forEachIndexed { index, id ->
                val bounds = positions.getOrNull(index)
                val body = measured[id]
                // A truncated placeholder has no rectangle and must not draw or receive taps.
                if (bounds != null && body != null)
                    body.place(bounds.left.roundToInt(), bounds.top.roundToInt())
            }
        }
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
    val customEmojis = workspaceEmojis(app)
    var query by remember(initialSearch) { mutableStateOf(initialSearch) }
    var emojis by remember(app.env.locale) { mutableStateOf<List<Row>>(emptyList()) }
    var recents by remember { mutableStateOf<List<String>>(emptyList()) }
    var groups by remember(app.env.locale) { mutableStateOf<List<Row>>(emptyList()) }
    var loading by remember(app.env.locale) { mutableStateOf(true) }
    var tone by remember { mutableStateOf("") }
    var kitchen by remember(initialKitchen) { mutableStateOf(initialKitchen) }
    var custom by remember { mutableStateOf(false) }
    LaunchedEffect(app.env.locale) {
        try {
            val locale =
                when (app.env.locale) {
                    "ja" -> "ja"
                    "ko-KR",
                    "ko-KP" -> "ko"
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
                            .then(webControlModifier("tab", selected = !custom && kitchen == tab))
                            .background(
                                if (!custom && kitchen == tab) MaterialTheme.colorScheme.background
                                else androidx.compose.ui.graphics.Color.Transparent,
                                androidx.compose.foundation.shape.RoundedCornerShape(6.dp),
                            ),
                ) {
                    kitchen = tab
                    custom = false
                }
            }
            Button(app.t("home.customEmojis"), modifier = Modifier.weight(1f).then(webControlModifier("tab", selected = custom))) { custom = true; kitchen = false }
        }
        if (kitchen) {
            KitchenPicker(app, ::choose, kitchenCatalogUrl)
            return@pickerContent
        }
        if (custom) {
        Field(query, app.t("home.searchEmoji"), Modifier.fillMaxWidth()) { query = it }
        val accountMatches = app.accountProfiles.filter { (it.text("username") + " " + it.text("display_name")).contains(query, ignoreCase = true) }
        FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            accountMatches.forEach { account ->
                val value = ":@${account.text("username")}:"
                Box(Modifier.height(36.dp).clickable(role = androidx.compose.ui.semantics.Role.Button) { choose(value) }.semantics { contentDescription = value }) { Avatar(account, size = 28.dp) }
            }
        }
        val customMatches = customEmojis.filter { (listOf(it.text("name"), it.text("category")) + it.array("aliases").map { alias -> alias.jsonPrimitive.content }).any { text -> text.contains(query, ignoreCase = true) } }
        if (customMatches.isNotEmpty()) {
            Text(app.t("home.customEmojis"), small = true)
            FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                customMatches.groupBy { it.text("category") }.forEach { (category, items) ->
                    Text(category.ifEmpty { app.t("customEmoji.uncategorized") }, Modifier.fillMaxWidth(), small = true)
                    items.forEach { emoji ->
                    val value = ":${emoji.text("name")}:"
                    Box(Modifier.height(36.dp).clickable(role = androidx.compose.ui.semantics.Role.Button) { choose(value) }.semantics { contentDescription = value }) { ActivityPubEmoji(workspaceEmoji(emoji), 28.dp) }
                    }
                }
            }
        }
        if (app.workspaceEmojiLoading) Text(app.t("home.loading"), Modifier.then(webControlModifier("status")))
        else if (app.workspaceEmojiError) {
            Text(app.t("home.controlsError"), Modifier.then(webControlModifier("alert")))
            Button(app.t("home.refresh")) { app.run { app.loadWorkspaceEmojis(true) } }
        } else if (customMatches.isEmpty() && accountMatches.isEmpty()) Text(app.t("home.noEmojis"))
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
