package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.ui.platform.testTag
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private enum class ShareScope(val label: String) { All("すべて"), Channels("チャンネル"), Direct("DM") }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ShareDestinationSheet(
    text: String, imageCount: Int, sections: List<ChannelSection>, colors: AndroidColors, busy: Boolean, error: String?,
    cancel: () -> Unit, preview: @Composable () -> Unit = {}, select: (String?) -> Unit,
) {
    var query by rememberSaveable { mutableStateOf("") }
    val searchable = sections.sumOf { it.channels.size } > 6
    val term = query.trim().takeIf { searchable }.orEmpty()
    val hasBothTypes = searchable && sections.any { it.channels.any(ChannelItem::direct) } &&
        sections.any { section -> section.channels.any { !it.direct } }
    var selectedScope by rememberSaveable { mutableStateOf(ShareScope.All) }
    val scope = if (hasBothTypes) selectedScope else ShareScope.All
    val showHome = scope == ShareScope.All && "ホーム".contains(term)
    val matches = remember(sections, term, scope) {
        sections.map { section -> section.copy(channels = section.channels.filter {
            (scope == ShareScope.All || it.direct == (scope == ShareScope.Direct)) &&
                (it.title.contains(term, ignoreCase = true) || it.handle?.contains(term, ignoreCase = true) == true)
        }) }.filter { it.channels.isNotEmpty() }
    }
    val list = rememberLazyListState()
    ModalBottomSheet(
        onDismissRequest = { if (!busy) cancel() }, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = colors.background, contentColor = colors.foreground,
        dragHandle = { BottomSheetDefaults.DragHandle(color = colors.inputBoundary) },
    ) {
        LazyColumn(Modifier.fillMaxWidth().then(if (searchable) Modifier.fillMaxHeight() else Modifier)
            .imePadding().testTag("share-targets"), state = list, contentPadding = PaddingValues(bottom = 16.dp)) {
            item {
                Column(Modifier.padding(horizontal = 16.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("共有先", Modifier.weight(1f), color = colors.foreground, fontWeight = AndroidWeights.emphasis, fontSize = 22.sp)
                        UiIconButton(UiIcon.Close, "閉じる", colors.secondary, cancel, enabled = !busy, modifier = Modifier.size(36.dp))
                    }
                    Column(Modifier.fillMaxWidth().padding(vertical = 12.dp)
                        .background(colors.inputSurface, RoundedCornerShape(12.dp)).padding(12.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        if (imageCount > 0) preview()
                        if (text.isNotBlank()) Text(text, color = colors.foreground, fontSize = 15.sp, maxLines = 3, overflow = TextOverflow.Ellipsis)
                        if (imageCount > 0) Text("画像 $imageCount 件", color = colors.secondary, fontSize = 12.sp)
                    }
                    if (!searchable) {
                        error?.let { Text(it, color = colors.unread, fontSize = 14.sp) }
                        ProgressStrip(busy, colors.foreground)
                    }
                }
            }
            if (searchable) stickyHeader(key = "search") {
                Column(Modifier.fillMaxWidth().background(colors.background)) {
                    SearchInput(query, {
                        query = it
                        list.requestScrollToItem(1)
                    }, "共有先を検索", colors, Modifier.padding(horizontal = 16.dp, vertical = 4.dp).testTag("share-search"))
                    if (hasBothTypes) Row(Modifier.selectableGroup().padding(horizontal = 16.dp),
                        horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        ShareScope.entries.forEach { option ->
                            ScopeTab(option.label, scope == option, colors) {
                                selectedScope = option
                                list.requestScrollToItem(1)
                            }
                        }
                    }
                    error?.let { Text(it, Modifier.padding(horizontal = 16.dp), color = colors.unread, fontSize = 14.sp) }
                    ProgressStrip(busy, colors.foreground)
                }
            }
            if (showHome) item("home") {
                ShareTarget("ホーム", UiIcon.Home, colors, busy) { select(null) }
            }
            for (section in matches) {
                item { Text(section.label, Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 4.dp), color = colors.secondary, fontSize = 13.sp) }
                items(section.channels, key = { "channel-" + it.id }) {
                    ShareTarget(it.title, if (it.direct) UiIcon.Person else UiIcon.Channels, colors, busy, it.handle) { select(it.id) }
                }
            }
            if (term.isNotEmpty() && matches.isEmpty() && !showHome)
                item { Text("共有先が見つかりません", Modifier.padding(horizontal = 16.dp, vertical = 12.dp), color = colors.secondary) }
        }
    }
}

@Composable
private fun ShareTarget(label: String, icon: UiIcon, colors: AndroidColors, busy: Boolean, handle: String? = null, click: () -> Unit) {
    LabeledRow(label, handle?.takeIf { it != label }, colors,
        Modifier.clickable(enabled = !busy, role = Role.Button, onClickLabel = "下書きを開く", onClick = click),
        leading = {
            Box(Modifier.size(32.dp).background(if (icon == UiIcon.Home) colors.headerLight else colors.inputSurface,
                if (icon == UiIcon.Person) CircleShape else RoundedCornerShape(9.dp)), contentAlignment = Alignment.Center) {
                UiGlyph(icon, colors.foreground, 17.dp)
            }
        }) { UiGlyph(UiIcon.Next, colors.secondary, AndroidControlSize.inlineIcon) }
}
