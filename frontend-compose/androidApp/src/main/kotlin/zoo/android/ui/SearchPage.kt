package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
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
import zoo.searchTerms

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
)

@Composable
internal fun ColumnScope.SearchPage(screen: AndroidScreen.Search, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit) {
    val state = screen.state
    val title = androidLabel("globalSearch.title", "検索")
    PageHeader(title, colors, onBack = { dispatch(AndroidUiAction.Back) })
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        SearchField(state.text, colors) { dispatch(AndroidUiAction.SearchMessages(it)) }
        Row(Modifier.fillMaxWidth().selectableGroup()) {
            for (kind in GlobalSearchType.entries) {
                val selected = kind == state.type
                Box(
                    Modifier.weight(1f).heightIn(min = AndroidControlSize.target).clip(CircleShape)
                        .background(if (selected) colors.inputSurface else Color.Transparent)
                        .selectable(selected, role = Role.Tab) { dispatch(AndroidUiAction.SelectSearchType(kind)) },
                    contentAlignment = Alignment.Center,
                ) {
                    Text(androidLabel(kind.labelKey, kind.fallback), color = if (selected) colors.foreground else colors.secondary,
                        fontSize = 15.sp, fontWeight = if (selected) AndroidWeights.emphasis else AndroidWeights.label,
                        maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        Text(androidLabel("globalSearch.encryptedNote", "暗号化されたチャンネルは検索の対象外です。"),
            color = colors.secondary, fontSize = 13.sp)
    }
    Box(Modifier.weight(1f).fillMaxWidth()) {
        when {
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
            else -> {
                val highlight = remember(state.query, colors.searchHighlight) {
                    EntryHighlight(searchTerms(state.query),
                        SpanStyle(color = colors.searchHighlightText, background = colors.searchHighlight))
                }
                LazyColumn(
                    Modifier.fillMaxSize().testTag("global-search-results"),
                    rememberLazyListState(),
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
                ) {
                    items(state.results, key = { it.key }) { item ->
                        SearchResult(item, colors, highlight) { dispatch(AndroidUiAction.OpenSearchResult(item.key)) }
                    }
                    item("search-footer") { SearchFooter(state, colors, dispatch) }
                }
            }
        }
    }
}

@Composable
private fun SearchField(text: String, colors: AndroidColors, change: (String) -> Unit) {
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
private fun SearchResult(item: SearchResultItem, colors: AndroidColors, highlight: EntryHighlight, open: () -> Unit) {
    val thread = androidLabel("globalSearch.inThread", "スレッドの返信")
    val destination = listOfNotNull(item.destination, thread.takeIf { item.inThread }).joinToString(" · ")
    Row(
        Modifier.fillMaxWidth().testTag("global-search-result")
            .clickable(role = Role.Button, onClick = open)
            .semantics(mergeDescendants = true) {}
            .padding(vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(Modifier.size(36.dp).background(colors.quietSurface, CircleShape), contentAlignment = Alignment.Center) {
            Text(item.author.take(1), color = colors.secondary, fontSize = 13.sp)
        }
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
                val snippet = remember(item.body, highlight) { formatEntry(item.body, highlight = highlight) }
                Text(snippet, color = colors.foreground, fontSize = entryTextSize, lineHeight = 23.sp,
                    maxLines = 3, overflow = TextOverflow.Ellipsis)
            }
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

private val GlobalSearchType.fallback: String
    get() = when (this) {
        GlobalSearchType.All -> "すべて"
        GlobalSearchType.Messages -> "メッセージ"
        GlobalSearchType.Files -> "ファイル"
    }
