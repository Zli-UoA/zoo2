package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.ui.platform.testTag
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ShareDestinationSheet(
    text: String, imageCount: Int, sections: List<ChannelSection>, colors: AndroidColors, busy: Boolean, error: String?,
    cancel: () -> Unit, preview: @Composable () -> Unit = {}, select: (String?) -> Unit,
) {
    var query by rememberSaveable { mutableStateOf("") }
    val searchable = sections.sumOf { it.channels.size } > 6
    val term = query.trim().takeIf { searchable }.orEmpty()
    val matches = remember(sections, term) {
        sections.map { section -> section.copy(channels = section.channels.filter {
            it.title.contains(term, ignoreCase = true) || it.handle?.contains(term, ignoreCase = true) == true
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
                Column(Modifier.padding(horizontal = 20.dp)) {
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
                    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 4.dp)
                        .background(colors.inputSurface, RoundedCornerShape(12.dp)).heightIn(min = 44.dp)
                        .padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        UiGlyph(UiIcon.Search, colors.secondary, 18.dp)
                        BasicTextField(query, {
                            query = it
                            list.requestScrollToItem(1)
                        }, Modifier.weight(1f).heightIn(min = 44.dp).testTag("share-search").semantics { contentDescription = "共有先を検索" }, singleLine = true,
                            textStyle = TextStyle(color = colors.foreground, fontSize = 16.sp), cursorBrush = SolidColor(colors.foreground),
                            decorationBox = { field -> Box(contentAlignment = Alignment.CenterStart) {
                                if (query.isEmpty()) Text("共有先を検索", color = colors.secondary)
                                field()
                            } })
                        if (query.isNotEmpty()) UiIconButton(UiIcon.Close, "検索をクリア", colors.secondary, {
                            query = ""
                            list.requestScrollToItem(1)
                        }, iconSize = 16.dp, modifier = Modifier.size(32.dp))
                    }
                    error?.let { Text(it, Modifier.padding(horizontal = 20.dp), color = colors.unread, fontSize = 14.sp) }
                    ProgressStrip(busy, colors.foreground)
                }
            }
            if ("ホーム".contains(term)) item("home") {
                ShareTarget("ホーム", UiIcon.Home, colors, busy) { select(null) }
            }
            for (section in matches) {
                item { Text(section.label, Modifier.padding(start = 20.dp, end = 20.dp, top = 12.dp, bottom = 4.dp), color = colors.secondary, fontSize = 13.sp) }
                items(section.channels, key = { "channel-" + it.id }) {
                    ShareTarget(it.title, if (it.direct) UiIcon.Person else UiIcon.Channels, colors, busy, it.handle) { select(it.id) }
                }
            }
            if (term.isNotEmpty() && matches.isEmpty() && !"ホーム".contains(term))
                item { Text("共有先が見つかりません", Modifier.padding(horizontal = 20.dp, vertical = 12.dp), color = colors.secondary) }
        }
    }
}

@Composable
private fun ShareTarget(label: String, icon: UiIcon, colors: AndroidColors, busy: Boolean, handle: String? = null, click: () -> Unit) {
    Row(Modifier.fillMaxWidth().clickable(enabled = !busy, role = Role.Button, onClickLabel = "下書きを開く", onClick = click)
        .heightIn(min = 48.dp).padding(horizontal = 20.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(Modifier.size(32.dp).background(if (icon == UiIcon.Home) colors.headerLight else colors.inputSurface,
            if (icon == UiIcon.Person) CircleShape else RoundedCornerShape(9.dp)), contentAlignment = Alignment.Center) {
            UiGlyph(icon, colors.foreground, 17.dp)
        }
        Column(Modifier.weight(1f)) {
            Text(label, color = colors.foreground, fontSize = 16.sp)
            if (handle != null && handle != label) Text(handle, color = colors.secondary, fontSize = 13.sp)
        }
        UiGlyph(UiIcon.Next, colors.secondary, 16.dp)
    }
}
