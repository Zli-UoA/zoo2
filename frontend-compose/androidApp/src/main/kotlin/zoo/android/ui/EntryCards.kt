package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
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
) : EntryContentBlock

internal data class EntryRichContent(
    val blocks: List<EntryContentBlock> = emptyList(),
    val hidesBody: Boolean = false,
)

@Composable
internal fun EntryBlocks(
    blocks: List<EntryContentBlock>, colors: AndroidColors, fontSize: TextUnit, emoji: Map<String, TimelineMedia>,
) {
    blocks.forEach { block ->
        when (block) {
            is EntryCard -> EntryCards(listOf(block), colors)
            is EntryContentText -> EntryBody(block.body, colors.foreground, fontSize, emoji)
            is EntryContentPanel -> Column(
                Modifier.padding(vertical = 4.dp).fillMaxWidth()
                    .background(colors.inputSurface, RoundedCornerShape(12.dp)).padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                if (block.caption.isNotBlank()) Text(entryLabel(block.caption, block.url), color = colors.secondary, fontSize = 12.sp)
                EntryBlocks(block.blocks, colors, 14.sp, emoji)
            }
        }
    }
}

@Composable
internal fun EntryCards(cards: List<EntryCard>, colors: AndroidColors) {
    if (cards.isEmpty()) return
    Column(Modifier.padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        cards.forEach { card ->
            Column(
                Modifier.fillMaxWidth().background(colors.inputSurface, RoundedCornerShape(12.dp)).padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                card.image?.let { media ->
                    val load = LocalTimelineImage.current
                    val bitmap by produceState<ImageBitmap?>(null, media, load) { value = load(media) }
                    bitmap?.let {
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
