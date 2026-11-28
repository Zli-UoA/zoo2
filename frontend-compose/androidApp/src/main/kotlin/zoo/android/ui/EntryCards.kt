package zoo.android.ui

import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class EntryCardField(val title: String, val value: String, val code: Boolean = false, val url: String? = null)

internal sealed interface EntryContentBlock

internal data class EntryContentText(val body: String) : EntryContentBlock

internal data class EntryContentPanel(
    val blocks: List<EntryContentBlock>,
    val caption: String = "",
    val url: String? = null,
) : EntryContentBlock

internal data class EntryCard(
    val title: String,
    val description: String,
    val url: String? = null,
    val caption: String = "",
    val fields: List<EntryCardField> = emptyList(),
    val image: TimelineMedia? = null,
    val formatted: Boolean = true,
    val summaly: Boolean = false,
) : EntryContentBlock

internal data class EntryRichContent(
    val blocks: List<EntryContentBlock> = emptyList(),
    val hidesBody: Boolean = false,
)

internal fun EntryContentBlock.displayedText(): Sequence<String> = when (this) {
    is EntryContentText -> sequenceOf(body)
    is EntryContentPanel -> blocks.asSequence().flatMap { it.displayedText() }
    is EntryCard -> emptySequence()
}

@Composable
internal fun EntryBlocks(
    blocks: List<EntryContentBlock>, colors: AndroidColors, fontSize: TextUnit, emoji: Map<String, TimelineMedia>,
) {
    blocks.forEach { block ->
        when (block) {
            is EntryCard -> EntryCards(listOf(block), colors)
            // 連携の投稿の本文は、アプリ内リンクを短くせず、行き先も取らない（docs/mfm.md）。
            is EntryContentText -> EntryBody(block.body, colors.foreground, fontSize, emoji, appLinks = false)
            is EntryContentPanel -> Column(
                Modifier.padding(vertical = 4.dp).fillMaxWidth()
                    .background(colors.inputSurface, RoundedCornerShape(12.dp)).padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                if (block.caption.isNotBlank()) Text(entryLabel(block.caption, block.url), color = colors.secondary, fontSize = 12.sp)
                EntryBlocks(block.blocks, colors, fontSize, emoji)
            }
        }
    }
}

@Composable
internal fun EntryCards(cards: List<EntryCard>, colors: AndroidColors, open: ((String) -> Unit)? = null) {
    if (cards.isEmpty()) return
    Column(Modifier.padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        cards.forEach { card ->
            if (card.summaly) SummalyCard(card, colors, open)
            else Column(
                Modifier.fillMaxWidth().background(colors.inputSurface, RoundedCornerShape(12.dp)).padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                card.image?.let { media ->
                    rememberTimelineImage(media)?.let {
                        Image(it, contentDescription = null, Modifier.fillMaxWidth().heightIn(max = 128.dp),
                            contentScale = ContentScale.Crop)
                    }
                }
                if (card.caption.isNotBlank()) Text(card.caption, color = colors.secondary, fontSize = 12.sp)
                Text(entryLabel(card.title, card.url), color = colors.foreground,
                    fontSize = 15.sp, fontWeight = AndroidWeights.emphasis)
                if (card.description.isNotBlank()) {
                    val description = if (card.formatted) formatEntry(card.description) else entryLabel(card.description, null)
                    Text(description, color = colors.foreground, fontSize = 14.sp)
                }
                card.fields.forEach { field ->
                    if (field.title.isNotBlank()) Text(entryLabel(field.title, field.url), color = colors.secondary,
                        fontSize = 12.sp, fontWeight = AndroidWeights.emphasis)
                    val value = if (field.code) entryCode(field.value) else formatEntry(field.value)
                    Text(value, color = colors.foreground, fontSize = 14.sp)
                }
            }
        }
    }
}

@Composable
private fun SummalyCard(card: EntryCard, colors: AndroidColors, open: ((String) -> Unit)?) {
    val url = card.url?.let({ zoo.publicUrlString(it) })
    val host = url?.let { android.net.Uri.parse(it).host }.orEmpty()
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(colors.background).border(1.dp, colors.inputBoundary, shape)
        .then(if (url != null && open != null) Modifier.clickable(role = Role.Button, onClickLabel = "記事を開く") {
            open(url)
        } else Modifier)) {
        card.image?.let { media ->
            val bitmap = rememberTimelineImage(media)
            Box(Modifier.fillMaxWidth().aspectRatio(16f / 9f).background(colors.inputSurface)) {
                bitmap?.let { Image(it, null, Modifier.matchParentSize(), contentScale = ContentScale.Crop) }
            }
        }
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                UiGlyph(UiIcon.External, colors.secondary, 14.dp)
                Text(listOf(card.caption, host).filter { it.isNotBlank() }.distinctBy { it.lowercase() }.joinToString(" · "),
                    color = colors.secondary, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Text(card.title.ifBlank { host }, color = colors.foreground, fontSize = 16.sp,
                fontWeight = AndroidWeights.emphasis, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (card.description.isNotBlank()) Text(card.description, color = colors.secondary, fontSize = 14.sp,
                maxLines = 3, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
internal fun PostLinkCard(url: String, colors: AndroidColors, open: () -> Unit) {
    val parsed = zoo.publicUrlString(url)?.let(android.net.Uri::parse) ?: return
    val shape = RoundedCornerShape(12.dp)
    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp).clip(shape).border(1.dp, colors.inputBoundary, shape)
        .clickable(role = Role.Button, onClickLabel = "投稿を開く", onClick = open).padding(12.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        UiGlyph(UiIcon.Bubble, colors.secondary, 18.dp)
        Column(Modifier.weight(1f)) {
            Text("Zoo の投稿", color = colors.foreground, fontSize = 15.sp, fontWeight = AndroidWeights.emphasis)
            Text(parsed.host + parsed.encodedPath, color = colors.secondary, fontSize = 12.sp,
                maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        UiGlyph(UiIcon.Next, colors.secondary, 16.dp)
    }
}
