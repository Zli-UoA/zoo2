package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class EmojiChoice(
    val value: String, val glyph: String, val name: String,
    val image: String? = null, val keywords: String = "",
)

internal class KitchenChoices(
    val components: List<EmojiChoice>,
    val combinations: (EmojiChoice) -> List<EmojiChoice>,
)

/**
 * Keeps only the choices matching every word of [query]. With a query, ranks (lower first):
 * 0 exact name, 1 name prefix, 2 name substring, then 3 exact / 4 prefix / 5 substring over the
 * keywords, and 6 anything else (e.g. the glyph). Shorter names first within a rank, then the
 * original order. An empty query keeps the original order.
 */
internal fun rankEmojiChoices(choices: List<EmojiChoice>, query: String): List<EmojiChoice> {
    val words = query.trim().replace('_', ' ').split(Regex("\\s+")).filter { it.isNotEmpty() }
    if (words.isEmpty()) return choices
    val whole = words.joinToString(" ")
    return choices.withIndex().mapNotNull { (index, choice) ->
        val searchable = "${choice.name} ${choice.glyph} ${choice.keywords.replace('_', ' ')}"
        if (!words.all { searchable.contains(it, ignoreCase = true) }) return@mapNotNull null
        val name = choice.name.replace('_', ' ').trim(':', ' ')
        val keywords = choice.keywords.replace('_', ' ').split(',', ' ').filter { it.isNotEmpty() }
        val rank = when {
            name.equals(whole, ignoreCase = true) -> 0
            name.startsWith(whole, ignoreCase = true) -> 1
            name.contains(whole, ignoreCase = true) -> 2
            keywords.any { it.equals(whole, ignoreCase = true) } -> 3
            keywords.any { it.startsWith(whole, ignoreCase = true) } -> 4
            keywords.any { it.contains(whole, ignoreCase = true) } -> 5
            else -> 6
        }
        Triple(rank, choice.name.length, index) to choice
    }.sortedWith(compareBy({ it.first.first }, { it.first.second }, { it.first.third })).map { it.second }
}

private enum class EmojiTab(val title: String) {
    Unicode("絵文字"), Custom("カスタム"), Kitchen("Emoji Kitchen"),
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun EmojiPicker(
    title: String,
    colors: AndroidColors,
    unicode: List<EmojiChoice>,
    custom: List<EmojiChoice>,
    recent: List<EmojiChoice> = emptyList(),
    customLoading: Boolean = false,
    customError: Boolean = false,
    loadKitchen: suspend () -> KitchenChoices?,
    loadImage: suspend (String) -> ImageBitmap?,
    dismiss: () -> Unit,
    select: (EmojiChoice) -> Unit,
) {
    var tab by remember { mutableStateOf(EmojiTab.Unicode) }
    var query by remember { mutableStateOf("") }
    var first by remember { mutableStateOf<EmojiChoice?>(null) }
    var kitchen by remember { mutableStateOf<KitchenChoices?>(null) }
    LaunchedEffect(tab) {
        if (tab == EmojiTab.Kitchen && kitchen == null) kitchen = loadKitchen()
    }
    val choices = remember(tab, unicode, custom, recent, kitchen, first) {
        when (tab) {
            EmojiTab.Unicode -> (recent.filter { it.image == null && it.glyph.isNotBlank() } + unicode)
                .distinctBy { it.value }
            EmojiTab.Custom -> custom
            EmojiTab.Kitchen -> kitchen?.let { catalog -> first?.let(catalog.combinations) ?: catalog.components }.orEmpty()
        }
    }
    val filtered = remember(choices, query) { rankEmojiChoices(choices, query) }
    ModalBottomSheet(
        onDismissRequest = dismiss,
        modifier = Modifier.semantics { paneTitle = title },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        sheetMaxWidth = Dp.Unspecified,
        containerColor = colors.background,
        contentColor = colors.foreground,
    ) {
        Column(Modifier.fillMaxWidth().heightIn(max = 520.dp).imePadding().padding(horizontal = 16.dp)) {
            PrimaryTabRow(
                selectedTabIndex = tab.ordinal, containerColor = colors.background, contentColor = colors.foreground,
                indicator = {
                    TabRowDefaults.PrimaryIndicator(Modifier.tabIndicatorOffset(tab.ordinal, matchContentSize = true),
                        color = colors.accent)
                },
            ) {
                EmojiTab.entries.forEach { choice ->
                    Tab(selected = choice == tab, onClick = { tab = choice; query = "" },
                        text = { Text(choice.title, fontSize = 12.sp, maxLines = 1) }, selectedContentColor = colors.foreground, unselectedContentColor = colors.secondary)
                }
            }
            BasicTextField(
                value = query,
                onValueChange = { query = it },
                modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp).testTag("emoji-search")
                    .semantics { contentDescription = "絵文字を検索" },
                singleLine = true,
                textStyle = LocalTextStyle.current.copy(color = colors.inputText, fontSize = 15.sp),
                cursorBrush = SolidColor(colors.inputText),
                decorationBox = { field ->
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
                            .background(colors.inputSurface, RoundedCornerShape(14.dp)).padding(horizontal = 14.dp, vertical = 8.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        UiGlyph(UiIcon.Search, colors.inputHint, AndroidControlSize.inlineIcon)
                        Box(Modifier.weight(1f)) {
                            if (query.isEmpty()) Text("絵文字を検索", color = colors.inputHint, fontSize = 15.sp)
                            field()
                        }
                    }
                },
            )
            if (tab == EmojiTab.Kitchen) {
                val prefix = first
                if (prefix == null) Text("最初の絵文字を選ぶ", color = colors.secondary)
                else TextAction("${prefix.glyph}  組み合わせを選ぶ", colors, style = TextActionStyle.Plain,
                    icon = UiIcon.Back) { first = null; query = "" }
            }
            if (filtered.isEmpty()) {
                val message = when {
                    tab == EmojiTab.Custom && customLoading || tab == EmojiTab.Kitchen && kitchen == null -> "絵文字を読み込み中"
                    tab == EmojiTab.Custom && customError -> "カスタム絵文字の読み込みに失敗しました"
                    query.isNotBlank() -> "一致する絵文字がありません"
                    else -> "選べる絵文字がありません"
                }
                Text(message, Modifier.padding(vertical = 20.dp), color = colors.secondary)
            } else key(tab, first, query) {
                LazyVerticalGrid(GridCells.Adaptive(AndroidControlSize.target), Modifier.weight(1f, fill = false),
                    contentPadding = PaddingValues(bottom = 16.dp)) {
                    items(filtered, key = { it.value }) { choice ->
                        TooltipBox(
                            positionProvider = TooltipDefaults.rememberPlainTooltipPositionProvider(),
                            tooltip = { PlainTooltip { Text(choice.name) } },
                            state = rememberTooltipState(),
                        ) {
                        Box(Modifier.fillMaxWidth().height(AndroidControlSize.target).clickable(role = Role.Button) {
                            if (tab == EmojiTab.Kitchen && first == null) {
                                first = choice
                                query = ""
                            } else select(choice)
                        }.semantics { contentDescription = choice.name }, contentAlignment = Alignment.Center) {
                            val image by produceState<ImageBitmap?>(null, choice.image) {
                                value = choice.image?.let { loadImage(it) }
                            }
                            val bitmap = image
                            if (bitmap != null) Image(bitmap, null, Modifier.size(32.dp))
                            else if (choice.glyph.isNotBlank()) Text(choice.glyph, color = colors.foreground, fontSize = 22.sp)
                            else Text(choice.name, color = colors.foreground, fontSize = 10.sp, maxLines = 1)
                        }
                        }
                    }
                }
            }
        }
    }
}
