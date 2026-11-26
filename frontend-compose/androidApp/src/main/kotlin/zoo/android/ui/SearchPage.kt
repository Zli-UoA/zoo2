package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import zoo.GlobalSearchType
import zoo.SearchQueryProblem
import kotlinx.coroutines.flow.collectLatest
import zoo.searchTerms

internal val LocalSearchPreviewLoader = staticCompositionLocalOf<suspend (List<String>) -> Unit> { {} }

/** One `/api/search` hit, already resolved to display text by the adapter. */
internal data class SearchResultItem(
    val key: String,
    val author: String,
    val body: String,
    val time: String,
    /** The channel name without `#`, or the DM's other members. */
    val destination: String,
    val dm: Boolean = false,
    val inThread: Boolean = false,
    val file: Boolean = false,
    val fileSize: String? = null,
    /** Custom emoji in the snippet, drawn as in conversations. */
    val emoji: Map<String, TimelineMedia> = emptyMap(),
    val avatar: String? = null,
    val media: TimelineMedia? = null,
    val link: String? = null,
    val linkCard: EntryCard? = null,
)

/**
 * `/search`: `text` is the field, `query` the searched (debounced) words. Older months are only
 * read on request, so the footer names the searched month and offers the next step.
 */
internal data class SearchScreenState(
    val text: String,
    val query: String,
    val type: GlobalSearchType,
    val results: List<SearchResultItem> = emptyList(),
    val loading: Boolean = false,
    val loadingOlder: Boolean = false,
    val error: Boolean = false,
    val loaded: Boolean = false,
    val canLoadOlder: Boolean = false,
    val searchedUntil: String? = null,
    /** The words break the server's limits: nothing was sent, and a retry would not pass. */
    val problem: SearchQueryProblem? = null,
    val filters: SearchFilterState = SearchFilterState(),
)

@Composable
internal fun ColumnScope.SearchPage(screen: AndroidScreen.Search, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    val state = screen.state
    val title = androidLabel("globalSearch.title", "検索")
    // A top tab like Channels: no back arrow, the navigation below switches pages.
    PageHeader(title, colors)
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        SearchField(state.text, colors, submit = { dispatch(AndroidUiAction.SubmitSearch) }) {
            dispatch(AndroidUiAction.SearchMessages(it))
        }
        SearchFilterRow(state.filters, state.type, colors, dispatch)
        Text(androidLabel("globalSearch.encryptedNote", "暗号化されたチャンネルは検索の対象外です。"),
            color = colors.secondary, fontSize = 13.sp)
    }
    // The list position is saved with its words, kind and filters (the screen's saved state has one key for
    // all searches): returning to the same search restores it, another search starts at the top.
    val positionKey = with(state.filters) {
        listOf(state.type.value, state.query, from.joinToString(","), inChannels.joinToString(","), "$mine", "$noBots")
            .joinToString("\n")
    }
    var savedKey by rememberSaveable { mutableStateOf("") }
    var savedIndex by rememberSaveable { mutableIntStateOf(0) }
    var savedOffset by rememberSaveable { mutableIntStateOf(0) }
    LaunchedEffect(positionKey) {
        // Another search (even one without a list, such as no results) forgets the position; returning
        // from an opened result keeps the same key and so keeps it.
        if (savedKey != positionKey) {
            savedKey = ""
            savedIndex = 0
            savedOffset = 0
        }
    }
    Box(Modifier.weight(1f).fillMaxWidth()) {
        when {
            state.problem != null -> SearchMessage(problemText(state.problem), colors)
            state.query.isEmpty() -> SearchMessage(
                androidLabel("globalSearch.prompt", "検索する語を入力してください。空白で区切ると、すべての語を含むものを探します。"), colors)
            state.loading -> SearchLoading(colors, Modifier.fillMaxSize())
            state.error && !state.loaded -> SearchMessage(androidLabel("globalSearch.loadError", "検索できませんでした。"), colors) {
                TextAction(androidLabel("globalSearch.retry", "再試行"), colors, style = TextActionStyle.Neutral, icon = UiIcon.Refresh) {
                    dispatch(AndroidUiAction.RetrySearch)
                }
            }
            state.results.isEmpty() && !state.canLoadOlder && !state.error ->
                SearchMessage(androidLabel("globalSearch.empty", "見つかりませんでした。"), colors)
            // Kept apart from the list: results read later must start at the newest, not where the footer was.
            state.results.isEmpty() -> Box(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
                SearchFooter(state, colors, dispatch)
            }
            else -> {
                val scroll = remember(positionKey) {
                    if (savedKey == positionKey) LazyListState(savedIndex, savedOffset) else LazyListState()
                }
                LaunchedEffect(scroll) {
                    snapshotFlow { scroll.firstVisibleItemIndex to scroll.firstVisibleItemScrollOffset }.collect { (index, offset) ->
                        savedKey = positionKey
                        savedIndex = index
                        savedOffset = offset
                    }
                }
                val loadPreviews = LocalSearchPreviewLoader.current
                val links = state.results.map { it.link }
                LaunchedEffect(scroll, links, loadPreviews) {
                    snapshotFlow {
                        scroll.layoutInfo.visibleItemsInfo.mapNotNull { links.getOrNull(it.index) }.distinct()
                    }.collectLatest(loadPreviews)
                }
                val highlight = remember(state.query, colors.searchHighlight) {
                    EntryHighlight(searchTerms(state.query),
                        SpanStyle(color = colors.searchHighlightText, background = colors.searchHighlight))
                }
                LazyColumn(
                    Modifier.fillMaxSize().testTag("global-search-results"),
                    scroll,
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
                ) {
                    items(state.results, key = { it.key }) { item ->
                        SearchResult(item, colors, highlight,
                            open = { dispatch(AndroidUiAction.OpenSearchResult(item.key)) },
                            openMedia = { dispatch(AndroidUiAction.OpenSearchMedia(item.key)) })
                    }
                    item("search-footer") { SearchFooter(state, colors, dispatch) }
                }
            }
        }
    }
}

@Composable
private fun SearchField(text: String, colors: AndroidColors, submit: () -> Unit, change: (String) -> Unit) {
    val focus = remember { FocusRequester() }
    // Returning from an opened result keeps the words and should not reopen the keyboard.
    val focusOnOpen = remember { text.isEmpty() }
    LaunchedEffect(Unit) { if (focusOnOpen) runCatching { focus.requestFocus() } }
    val placeholder = androidLabel("globalSearch.placeholder", "メッセージとファイルを検索")
    BasicTextField(
        text, change,
        Modifier.fillMaxWidth().focusRequester(focus).testTag("global-search-field")
            .semantics { contentDescription = placeholder },
        singleLine = true,
        textStyle = LocalTextStyle.current.copy(color = colors.inputText, fontSize = 16.sp),
        cursorBrush = SolidColor(colors.inputText),
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        keyboardActions = KeyboardActions(onSearch = { submit() }),
        decorationBox = { field ->
            Row(
                Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
                    .background(colors.inputSurface, CircleShape).padding(horizontal = 14.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                UiGlyph(UiIcon.Search, colors.inputHint, iconSize = AndroidControlSize.inlineIcon)
                Box(Modifier.weight(1f)) {
                    if (text.isEmpty()) Text(placeholder, color = colors.inputHint, fontSize = 16.sp, maxLines = 1)
                    field()
                }
            }
        },
    )
}

@Composable
private fun SearchResult(
    item: SearchResultItem, colors: AndroidColors, highlight: EntryHighlight, open: () -> Unit, openMedia: () -> Unit,
) {
    val thread = androidLabel("globalSearch.inThread", "スレッドの返信")
    val destination = listOfNotNull(item.destination, thread.takeIf { item.inThread }).joinToString(" · ")
    Row(
        Modifier.fillMaxWidth().testTag("global-search-result")
            .clickable(role = Role.Button, onClick = open)
            .semantics(mergeDescendants = true) {}
            .padding(vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        TimelineAvatar(item.author, item.avatar, colors)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(item.author, Modifier.weight(1f, fill = false), color = colors.foreground,
                    fontWeight = AndroidWeights.entryAuthor, fontSize = entryTextSize, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(item.time, color = colors.secondary, fontSize = 12.sp, maxLines = 1)
            }
            if (item.file) Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                UiGlyph(UiIcon.Attach, colors.secondary, iconSize = AndroidControlSize.inlineIcon)
                val name = remember(item.body, highlight) { buildAnnotatedString { appendHighlighted(item.body, highlight) } }
                Text(name, Modifier.weight(1f, fill = false), color = colors.foreground, fontSize = entryTextSize,
                    fontWeight = AndroidWeights.label, maxLines = 2, overflow = TextOverflow.Ellipsis)
                item.fileSize?.let { Text(it, color = colors.secondary, fontSize = 12.sp, maxLines = 1) }
            } else {
                // The snippet is MFM like Web's result row; only its text parts are marked.
                // Links in the snippet stay text: a tap anywhere on the row opens the result, not the link.
                EntryBody(item.body, colors.foreground, entryTextSize, item.emoji, maxLines = 3, highlight = highlight,
                    lineHeight = 23.sp, links = false)
            }
            item.media?.let { media -> TimelineMediaGroup(listOf(media), colors, apub = false) { openMedia() } }
            item.linkCard?.let { EntryCards(listOf(it), colors) }
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
                UiGlyph(if (item.dm) UiIcon.Person else UiIcon.Channels, colors.secondary,
                    iconSize = AndroidControlSize.metadataIcon)
                Text(destination, color = colors.secondary, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}

@Composable
private fun SearchFooter(state: SearchScreenState, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    val month = state.searchedUntil.orEmpty()
    Column(
        Modifier.fillMaxWidth().padding(vertical = 12.dp).semantics { liveRegion = LiveRegionMode.Polite },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (state.results.isEmpty() && state.canLoadOlder) Text(
            androidLabel("globalSearch.emptyOlder", "{{month}} までに見つかりませんでした。さらに古い期間を検索できます。")
                .replace("{{month}}", month),
            color = colors.foreground, textAlign = TextAlign.Center,
        )
        when {
            state.error -> {
                Text(androidLabel("globalSearch.loadError", "検索できませんでした。"), color = colors.secondary, fontSize = 13.sp)
                TextAction(androidLabel("globalSearch.retry", "再試行"), colors, style = TextActionStyle.Neutral, icon = UiIcon.Refresh) {
                    dispatch(AndroidUiAction.RetrySearch)
                }
            }
            state.canLoadOlder -> {
                if (month.isNotEmpty() && state.results.isNotEmpty()) Text(
                    androidLabel("globalSearch.searchedUntil", "{{month}} まで検索しました").replace("{{month}}", month),
                    color = colors.secondary, fontSize = 13.sp,
                )
                if (state.loadingOlder) SearchLoading(colors)
                else TextAction(androidLabel("globalSearch.searchOlder", "さらに古い結果を検索"), colors,
                    style = TextActionStyle.Neutral, icon = UiIcon.Search) { dispatch(AndroidUiAction.LoadOlderResults) }
            }
            month.isNotEmpty() && state.results.isNotEmpty() -> Text(
                androidLabel("globalSearch.searchedUntil", "{{month}} まで検索しました").replace("{{month}}", month),
                color = colors.secondary, fontSize = 13.sp,
            )
        }
    }
}

@Composable
private fun SearchLoading(colors: AndroidColors, modifier: Modifier = Modifier) {
    Row(
        modifier.heightIn(min = AndroidControlSize.target),
        horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(Modifier.size(20.dp), color = colors.secondary, strokeWidth = 2.dp)
        Text(androidLabel("globalSearch.loading", "検索中…"), color = colors.secondary)
    }
}

@Composable
private fun SearchMessage(text: String, colors: AndroidColors, action: (@Composable () -> Unit)? = null) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 32.dp, vertical = 48.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(text, color = colors.secondary, textAlign = TextAlign.Center)
        action?.invoke()
    }
}

@Composable
private fun problemText(problem: SearchQueryProblem): String = when (problem) {
    SearchQueryProblem.TooManyTerms -> androidLabel("globalSearch.tooManyTerms", "検索できる語は {{max}} 個までです。語を減らしてください。")
    SearchQueryProblem.TermTooLong -> androidLabel("globalSearch.termTooLong", "1 つの語は {{max}} 文字までです。語を短くしてください。")
}.replace("{{max}}", problem.max.toString())
