package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import zoo.GlobalSearchType
import zoo.MAX_SEARCH_FILTER_IDS

/** A sender or channel to filter by (Web's `SearchFilterOption`). */
internal data class SearchFilterOption(
    val id: String,
    val label: String,
    /** Shown after the name, also matched (`@user_name`). */
    val detail: String? = null,
    val mark: SearchFilterMark = SearchFilterMark.Initial,
)

/** Senders show their initial, as the results do; channels `#`, or a lock when private (as Web). */
internal enum class SearchFilterMark { Initial, Channel, PrivateChannel }

/** The filters of the search screen and the choices for the two pickers. */
internal data class SearchFilterState(
    val people: List<SearchFilterOption> = emptyList(),
    val channels: List<SearchFilterOption> = emptyList(),
    val from: List<String> = emptyList(),
    val inChannels: List<String> = emptyList(),
    val mine: Boolean = false,
    val noBots: Boolean = false,
)

internal enum class SearchFilterField { From, In }

internal enum class SearchFilterFlag { Mine, NoBots }

/** Rows shown at once in a picker; more are found with its field (Web's `SHOWN`). */
private const val SHOWN = 50

/**
 * The row of filters under the field (Web's kinds and filter group without the period): the kind opens a
 * menu ("all" by default), the senders and channels a sheet with a field and checkboxes, "involving me"
 * and "exclude bots" switch on each tap.
 */
@Composable
internal fun SearchFilterRow(
    state: SearchFilterState, type: GlobalSearchType, colors: AndroidColors, dispatch: (AndroidUiAction) -> Unit,
) {
    var open by rememberSaveable { mutableStateOf<SearchFilterField?>(null) }
    val fromLabel = androidLabel("globalSearch.filters.from", "送信者")
    val inLabel = androidLabel("globalSearch.filters.in", "チャンネル")
    val filtersLabel = androidLabel("globalSearch.filters.label", "絞り込み")
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())
            .semantics { contentDescription = filtersLabel },
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        TypeChip(type, colors) { dispatch(AndroidUiAction.SelectSearchType(it)) }
        FilterChip(summary(fromLabel, state.from, state.people), state.from.isNotEmpty(), UiIcon.Person, colors,
            Modifier.testTag("search-filter-from")) { open = SearchFilterField.From }
        FilterChip(summary(inLabel, state.inChannels, state.channels), state.inChannels.isNotEmpty(), UiIcon.Channels, colors,
            Modifier.testTag("search-filter-in")) { open = SearchFilterField.In }
        FilterToggle(androidLabel("globalSearch.filters.mine", "自分が参加中"), state.mine, colors) {
            dispatch(AndroidUiAction.ToggleSearchFilter(SearchFilterFlag.Mine))
        }
        FilterToggle(androidLabel("globalSearch.filters.noBots", "ボットを除く"), state.noBots, colors) {
            dispatch(AndroidUiAction.ToggleSearchFilter(SearchFilterFlag.NoBots))
        }
    }
    if (state.from.isNotEmpty() || state.inChannels.isNotEmpty()) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            for (field in listOf(SearchFilterField.From, SearchFilterField.In)) {
                val ids = if (field == SearchFilterField.From) state.from else state.inChannels
                val options = if (field == SearchFilterField.From) state.people else state.channels
                ids.forEach { id ->
                    val label = options.find { it.id == id }?.label ?: id
                    TextAction(label, colors, icon = UiIcon.Close, style = TextActionStyle.Neutral) {
                        dispatch(AndroidUiAction.SelectSearchFilter(field, ids - id))
                    }
                }
            }
        }
    }
    when (open) {
        SearchFilterField.From -> SearchFilterSheet(
            fromLabel, androidLabel("globalSearch.filters.searchPeople", "名前で探す"), state.people, state.from, colors,
            dismiss = { open = null },
        ) { dispatch(AndroidUiAction.SelectSearchFilter(SearchFilterField.From, it)) }
        SearchFilterField.In -> SearchFilterSheet(
            inLabel, androidLabel("globalSearch.filters.searchChannels", "チャンネル名で探す"), state.channels, state.inChannels,
            colors, dismiss = { open = null },
        ) { dispatch(AndroidUiAction.SelectSearchFilter(SearchFilterField.In, it)) }
        null -> {}
    }
}

/** "送信者", "送信者: はる" or "送信者: 2 件", as Web's picker button. */
@Composable
private fun summary(label: String, selected: List<String>, options: List<SearchFilterOption>): String {
    val first = options.firstOrNull { it.id == selected.firstOrNull() }
    return when {
        selected.isEmpty() -> label
        selected.size == 1 && first != null -> "$label: ${first.label}"
        else -> androidLabel("globalSearch.filters.selectedCount", "{{label}}: {{count}} 件")
            .replace("{{label}}", label).replace("{{count}}", selected.size.toString())
    }
}

/** A chip of the filter row: a filled face while it narrows the results, an outline otherwise. */
@Composable
private fun ChipFace(text: String, on: Boolean, icon: UiIcon?, colors: AndroidColors, modifier: Modifier) {
    Row(
        modifier.heightIn(min = 36.dp)
            .then(if (on) Modifier.background(colors.headerLight, CircleShape)
                else Modifier.border(1.dp, colors.inputBoundary, CircleShape))
            .padding(horizontal = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        icon?.let { UiGlyph(it, if (on) colors.foreground else colors.secondary, iconSize = AndroidControlSize.inlineIcon) }
        Text(text, Modifier.widthIn(max = 200.dp), color = colors.foreground, fontSize = 14.sp,
            fontWeight = if (on) AndroidWeights.emphasis else AndroidWeights.label, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun FilterChip(text: String, on: Boolean, icon: UiIcon, colors: AndroidColors, modifier: Modifier, click: () -> Unit) {
    // The chip opens the sheet; whether it narrows the results is its selected state, not only its colour.
    Box(modifier.heightIn(min = AndroidControlSize.target).clickable(role = Role.Button, onClick = click)
        .semantics { selected = on },
        contentAlignment = Alignment.Center) {
        ChipFace(text, on, icon, colors, Modifier)
    }
}

internal val GlobalSearchType.fallback: String
    get() = when (this) {
        GlobalSearchType.All -> "すべて"
        GlobalSearchType.Messages -> "メッセージ"
        GlobalSearchType.Files -> "ファイル"
    }

/**
 * The kind, as a chip showing the chosen one: "all" is the default and leaves it unfilled; the others fill
 * it like a filter in use. One kind at a time, so a menu instead of a sheet.
 */
@Composable
private fun TypeChip(type: GlobalSearchType, colors: AndroidColors, select: (GlobalSearchType) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        FilterChip(androidLabel(type.labelKey, type.fallback), type != GlobalSearchType.All, UiIcon.Document, colors,
            Modifier.testTag("search-filter-type")) { expanded = true }
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }, containerColor = colors.background) {
            Column(Modifier.selectableGroup()) {
                for (kind in GlobalSearchType.entries) {
                    val chosen = kind == type
                    DropdownMenuItem(
                        text = {
                            Text(androidLabel(kind.labelKey, kind.fallback), color = colors.foreground,
                                fontWeight = if (chosen) AndroidWeights.emphasis else AndroidWeights.label)
                        },
                        // The chosen kind keeps its check; the others keep the same indent.
                        leadingIcon = {
                            Box(Modifier.size(AndroidControlSize.inlineIcon)) {
                                if (chosen) UiGlyph(UiIcon.Check, colors.foreground, AndroidControlSize.inlineIcon)
                            }
                        },
                        onClick = { expanded = false; if (!chosen) select(kind) },
                        modifier = Modifier.semantics { selected = chosen; role = Role.RadioButton },
                    )
                }
            }
        }
    }
}

@Composable
private fun FilterToggle(text: String, on: Boolean, colors: AndroidColors, toggle: () -> Unit) {
    Box(Modifier.heightIn(min = AndroidControlSize.target).toggleable(on, role = Role.Switch) { toggle() },
        contentAlignment = Alignment.Center) {
        ChipFace(text, on, if (on) UiIcon.Check else null, colors, Modifier)
    }
}

/**
 * A field and checkboxes; several can be chosen (results from any of them), at most
 * [MAX_SEARCH_FILTER_IDS]. The chosen ones come first, as on Web.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SearchFilterSheet(
    title: String, hint: String, options: List<SearchFilterOption>, selected: List<String>, colors: AndroidColors,
    dismiss: () -> Unit, change: (List<String>) -> Unit,
) {
    var query by remember { mutableStateOf("") }
    val chosen = selected.toSet()
    val shown = remember(options, query, chosen) {
        val needle = query.trim()
        val matches = options.filter {
            needle.isEmpty() || it.label.contains(needle, ignoreCase = true) || it.detail?.contains(needle, ignoreCase = true) == true
        }
        (matches.filter { it.id in chosen } + matches.filterNot { it.id in chosen }).take(SHOWN)
    }
    val full = selected.size >= MAX_SEARCH_FILTER_IDS
    ModalBottomSheet(
        onDismissRequest = dismiss,
        modifier = Modifier.semantics { paneTitle = title },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        sheetMaxWidth = Dp.Unspecified,
        containerColor = colors.background,
        contentColor = colors.foreground,
    ) {
        Column(Modifier.fillMaxWidth().heightIn(max = 560.dp).imePadding().padding(horizontal = 16.dp)) {
            Text(title, color = colors.foreground, fontSize = 17.sp, fontWeight = AndroidWeights.emphasis)
            BasicTextField(
                query, { query = it },
                Modifier.fillMaxWidth().padding(vertical = 8.dp).testTag("search-filter-field")
                    .semantics { contentDescription = hint },
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
                            if (query.isEmpty()) Text(hint, color = colors.inputHint, fontSize = 15.sp)
                            field()
                        }
                    }
                },
            )
            if (full) Text(
                androidLabel("globalSearch.filters.max", "選べるのは {{count}} 件までです")
                    .replace("{{count}}", MAX_SEARCH_FILTER_IDS.toString()),
                color = colors.secondary, fontSize = 13.sp,
            )
            if (shown.isEmpty()) Text(androidLabel("globalSearch.filters.noMatches", "見つかりません"),
                Modifier.padding(vertical = 20.dp), color = colors.secondary)
            else LazyColumn(Modifier.weight(1f, fill = false), contentPadding = PaddingValues(bottom = 8.dp)) {
                items(shown, key = { it.id }) { option ->
                    val checked = option.id in chosen
                    val enabled = checked || !full
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
                            .toggleable(checked, enabled = enabled, role = Role.Checkbox) { on ->
                                change(if (on) selected + option.id else selected - option.id)
                            },
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Checkbox(checked, onCheckedChange = null, enabled = enabled,
                            colors = CheckboxDefaults.colors(checkedColor = colors.accent, checkmarkColor = androidx.compose.ui.graphics.Color.Black, uncheckedColor = colors.secondary))
                        Box(Modifier.size(28.dp), contentAlignment = Alignment.Center) {
                            when (option.mark) {
                                SearchFilterMark.PrivateChannel ->
                                    UiGlyph(UiIcon.Lock, colors.secondary, iconSize = AndroidControlSize.inlineIcon)
                                SearchFilterMark.Channel ->
                                    UiGlyph(UiIcon.Channels, colors.secondary, iconSize = AndroidControlSize.inlineIcon)
                                SearchFilterMark.Initial -> Box(Modifier.size(28.dp).background(colors.quietSurface, CircleShape),
                                    contentAlignment = Alignment.Center) {
                                    Text(option.label.take(1), color = colors.secondary, fontSize = 12.sp)
                                }
                            }
                        }
                        Text(option.label, Modifier.weight(1f, fill = false), color = colors.foreground, fontSize = 15.sp,
                            maxLines = 1, overflow = TextOverflow.Ellipsis)
                        option.detail?.let {
                            Text(it, Modifier.weight(1f, fill = false), color = colors.secondary, fontSize = 13.sp,
                                maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
            }
            if (selected.isNotEmpty()) TextAction(androidLabel("globalSearch.filters.clearSelection", "選択を外す"), colors,
                style = TextActionStyle.Neutral, icon = UiIcon.Close) { change(emptyList()) }
            Spacer(Modifier.height(16.dp))
        }
    }
}
