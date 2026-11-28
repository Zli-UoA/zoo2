package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
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
    ModalBottomSheet(
        onDismissRequest = { if (!busy) cancel() }, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = colors.background, contentColor = colors.foreground,
        dragHandle = { BottomSheetDefaults.DragHandle(color = colors.inputBoundary) },
    ) {
        LazyColumn(Modifier.fillMaxWidth().imePadding(), contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 16.dp)) {
            item {
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
                if (sections.sumOf { it.channels.size } > 6) {
                    Row(Modifier.fillMaxWidth().padding(bottom = 8.dp).background(colors.inputSurface, RoundedCornerShape(12.dp))
                        .padding(horizontal = 12.dp, vertical = 9.dp), verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        UiGlyph(UiIcon.Search, colors.secondary, 18.dp)
                        BasicTextField(query, { query = it }, Modifier.fillMaxWidth(), singleLine = true,
                            textStyle = TextStyle(color = colors.foreground, fontSize = 16.sp), cursorBrush = SolidColor(colors.foreground),
                            decorationBox = { field -> Box { if (query.isBlank()) Text("共有先を検索", color = colors.secondary); field() } })
                    }
                }
                error?.let { Text(it, color = colors.unread, fontSize = 14.sp) }
                ProgressStrip(busy, colors.foreground)
            }
            if ("ホーム".contains(query.trim(), true)) item("home") {
                ShareTarget("ホーム", UiIcon.Home, colors, busy) { select(null) }
            }
            val matches = sections.map { section -> section.copy(channels = section.channels.filter { it.title.contains(query.trim(), true) }) }
            for (section in matches.filter { it.channels.isNotEmpty() }) {
                item { Text(section.label, Modifier.padding(top = 12.dp, bottom = 4.dp), color = colors.secondary, fontSize = 13.sp) }
                items(section.channels, key = { "channel-" + it.id }) {
                    ShareTarget(it.title, if (it.direct) UiIcon.Person else UiIcon.Channels, colors, busy) { select(it.id) }
                }
            }
            if (query.isNotBlank() && matches.all { it.channels.isEmpty() } && !"ホーム".contains(query.trim(), true))
                item { Text("共有先が見つかりません", Modifier.padding(vertical = 12.dp), color = colors.secondary) }
        }
    }
}

@Composable
private fun ShareTarget(label: String, icon: UiIcon, colors: AndroidColors, busy: Boolean, click: () -> Unit) {
    Row(Modifier.fillMaxWidth().clickable(enabled = !busy, role = Role.Button, onClickLabel = "下書きを開く", onClick = click)
        .heightIn(min = 48.dp).padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(Modifier.size(32.dp).background(if (icon == UiIcon.Home) colors.headerLight else colors.inputSurface,
            if (icon == UiIcon.Person) CircleShape else RoundedCornerShape(9.dp)), contentAlignment = Alignment.Center) {
            UiGlyph(icon, colors.foreground, 17.dp)
        }
        Text(label, Modifier.weight(1f), color = colors.foreground, fontSize = 16.sp)
        UiGlyph(UiIcon.Next, colors.secondary, 16.dp)
    }
}
