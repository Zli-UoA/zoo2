package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class EmojiChoice(
    val value: String, val glyph: String, val name: String,
    val image: String? = null, val keywords: String = "", val category: String = "",
    val localizedName: String = "",
)

internal class KitchenChoices(
    val components: List<EmojiChoice>,
    val combinations: (EmojiChoice) -> List<EmojiChoice>,
)

/**
 * Keeps only the choices matching every word of [query]. With a query, ranks (lower first):
 * 0 exact name, 1 name prefix, 2 name substring (including the localized name), then
 * 3 exact / 4 prefix / 5 substring over the
 * keywords, and 6 anything else (e.g. the glyph). Shorter names first within a rank, then the
 * original order. An empty query keeps the original order.
 */
internal fun rankEmojiChoices(choices: List<EmojiChoice>, query: String): List<EmojiChoice> {
    val words = query.trim().replace('_', ' ').split(Regex("\\s+")).filter { it.isNotEmpty() }
    if (words.isEmpty()) return choices
    val whole = words.joinToString(" ")
    return choices.withIndex().mapNotNull { (index, choice) ->
        val names = listOf(choice.name, choice.localizedName).map { it.replace('_', ' ').trim(':', ' ') }
        val searchable = "${choice.name} ${choice.localizedName} ${choice.glyph} ${choice.keywords.replace('_', ' ')}"
        if (!words.all { searchable.contains(it, ignoreCase = true) }) return@mapNotNull null
        val keywords = choice.keywords.replace('_', ' ').split(',', ' ').filter { it.isNotEmpty() }
        val rank = when {
            names.any { it.equals(whole, ignoreCase = true) } -> 0
            names.any { it.startsWith(whole, ignoreCase = true) } -> 1
            names.any { it.contains(whole, ignoreCase = true) } -> 2
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
    recent: List<String> = emptyList(),
    recentCategory: String = "最近使った絵文字",
    customLoading: Boolean = false,
    customError: Boolean = false,
    retryCustom: (() -> Unit)? = null,
    loadKitchen: suspend () -> KitchenChoices?,
    loadImage: suspend (String) -> ImageBitmap?,
    dismiss: () -> Unit,
    prefetchCustomImages: suspend (List<String>) -> Unit = {},
    select: (EmojiChoice) -> Unit,
) {
    var tab by remember { mutableStateOf(EmojiTab.Unicode) }
    var query by remember { mutableStateOf("") }
    var prefetchCustom by remember { mutableStateOf(false) }
    val prefetch by rememberUpdatedState(prefetchCustomImages)
    val customImageUrls = remember(custom) { custom.mapNotNull { it.image }.distinct() }
    LaunchedEffect(prefetchCustom, customImageUrls) {
        if (prefetchCustom && customImageUrls.isNotEmpty()) prefetch(customImageUrls)
    }
    var first by remember { mutableStateOf<EmojiChoice?>(null) }
    var kitchen by remember { mutableStateOf<KitchenChoices?>(null) }
    var kitchenFailed by remember { mutableStateOf(false) }
    var attempt by remember { mutableIntStateOf(0) }
    LaunchedEffect(tab, attempt) {
        if (tab == EmojiTab.Kitchen && kitchen == null) {
            kitchenFailed = false
            kitchen = try { loadKitchen() } catch (e: kotlinx.coroutines.CancellationException) { throw e } catch (_: Exception) { null }
            kitchenFailed = kitchen == null
        }
    }
    val recentUnicode = remember(recent, unicode) {
        val catalog = unicode.associateBy { it.value }
        recent.mapNotNull(catalog::get)
    }
    val choices = remember(tab, unicode, custom, recentUnicode, kitchen, first) {
        when (tab) {
            EmojiTab.Unicode -> (recentUnicode + unicode)
                .distinctBy { it.value }
            EmojiTab.Custom -> custom
            EmojiTab.Kitchen -> kitchen?.let { catalog -> first?.let(catalog.combinations) ?: catalog.components }.orEmpty()
        }
    }
    val filtered = remember(choices, query) { rankEmojiChoices(choices, query) }
    val sections = remember(filtered, tab, query, unicode, recentUnicode, recentCategory) {
        when {
            tab == EmojiTab.Unicode && query.isNotBlank() -> {
                val matchesByCategory = filtered.groupBy { it.category }
                unicode.map { it.category }.distinct().mapNotNull { category ->
                    matchesByCategory[category]?.let { category to it }
                }
            }
            query.isNotBlank() || tab == EmojiTab.Kitchen -> listOf("" to filtered)
            tab == EmojiTab.Unicode -> buildList {
                if (recentUnicode.isNotEmpty()) add(recentCategory to recentUnicode)
                addAll(unicode.groupBy { it.category }.toList())
            }
            else -> filtered.groupBy { it.category }.toList()
        }
    }
    ActionSheet(colors, dismiss, Modifier.semantics { paneTitle = title }, maxWidth = Dp.Unspecified,
        handleModifier = Modifier.testTag("emoji-sheet-handle")) {
        Column(Modifier.fillMaxWidth().fillMaxHeight().imePadding().padding(horizontal = 16.dp)) {
            PrimaryTabRow(
                selectedTabIndex = tab.ordinal, containerColor = colors.background, contentColor = colors.foreground,
                indicator = {
                    TabRowDefaults.PrimaryIndicator(Modifier.tabIndicatorOffset(tab.ordinal, matchContentSize = true),
                        color = colors.accent)
                },
            ) {
                EmojiTab.entries.forEach { choice ->
                    Tab(selected = choice == tab, onClick = {
                        if (choice == EmojiTab.Custom) prefetchCustom = true
                        tab = choice
                        query = ""
                    },
                        text = { Text(choice.title, fontSize = 14.sp, maxLines = 1) }, selectedContentColor = colors.foreground, unselectedContentColor = colors.secondary)
                }
            }
            SearchInput(query, { query = it }, "絵文字を検索", colors,
                Modifier.padding(vertical = 8.dp).testTag("emoji-search"))
            if (tab == EmojiTab.Kitchen) {
                val prefix = first
                if (prefix == null) Text("最初の絵文字を選ぶ", color = colors.secondary)
                else TextAction("${prefix.glyph}  組み合わせを選ぶ", colors, style = TextActionStyle.Plain,
                    icon = UiIcon.Back) { first = null; query = "" }
            }
            if (filtered.isEmpty()) {
                val message = when {
                    tab == EmojiTab.Kitchen && kitchenFailed -> "組み合わせを取得できませんでした"
                    tab == EmojiTab.Custom && customLoading || tab == EmojiTab.Kitchen && kitchen == null -> "絵文字を読み込み中"
                    tab == EmojiTab.Custom && customError -> "カスタム絵文字の読み込みに失敗しました"
                    query.isNotBlank() -> "一致する絵文字がありません"
                    else -> "選べる絵文字がありません"
                }
                Text(message, Modifier.padding(vertical = 20.dp), color = colors.secondary)
                if (tab == EmojiTab.Kitchen && kitchenFailed) TextAction("再試行", colors, icon = UiIcon.Refresh) { attempt++ }
                if (tab == EmojiTab.Custom && customError && retryCustom != null) TextAction("再試行", colors, icon = UiIcon.Refresh, click = retryCustom)
            } else key(tab, first, query) {
                LazyVerticalGrid(GridCells.Adaptive(AndroidControlSize.target), Modifier.weight(1f, fill = false),
                    contentPadding = PaddingValues(bottom = 16.dp)) {
                    sections.forEachIndexed { index, (category, section) ->
                        if (category.isNotEmpty()) {
                            item(key = "category:$index:$category", span = { GridItemSpan(maxLineSpan) }) {
                                Text(category, Modifier.padding(top = 12.dp, bottom = 4.dp)
                                    .semantics { heading() }, color = colors.secondary,
                                    style = MaterialTheme.typography.titleSmall)
                            }
                        }
                        items(section, key = { "emoji:$index:${it.value}" }) { choice ->
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
                                    else if (choice.glyph.isNotBlank()) Text(choice.glyph, color = colors.foreground, fontSize = 24.sp)
                                    else Text(choice.name, color = colors.foreground, fontSize = 10.sp, maxLines = 1)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
