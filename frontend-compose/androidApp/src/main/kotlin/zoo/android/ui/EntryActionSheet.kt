package zoo.android.ui

import android.content.ClipData
import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun EntryActionSheet(
    entry: TimelineItem, colors: AndroidColors, posts: Boolean,
    dismiss: () -> Unit, open: (() -> Unit)?,
    action: (EntryOperation, String) -> Unit,
) {
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    ModalBottomSheet(onDismissRequest = dismiss, containerColor = colors.background) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(bottom = 16.dp)) {
            Text(entry.body, Modifier.padding(horizontal = 20.dp), color = colors.secondary, fontSize = entryTextSize, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Spacer(Modifier.height(12.dp))
            if (open != null) {
                SheetAction(
                    if (posts) UiIcon.Document else UiIcon.Bubble,
                    if (posts) "投稿を開く" else "スレッドを開く",
                    colors,
                    click = open,
                )
            }
            if (entry.editable) {
                SheetAction(UiIcon.Format, "編集", colors) { action(EntryOperation.Edit, "") }
                SheetAction(UiIcon.Delete, "削除", colors) { action(EntryOperation.Delete, "") }
            }
            if (entry.canReact) SheetAction(UiIcon.Emoji, "リアクション", colors, entry.operationStatus[EntryOperation.React] != EntryOperationStatus.Sending) { action(EntryOperation.React, "") }
            if (entry.canRepost) SheetAction(UiIcon.Repost, if (entry.reposted) "リノートを取り消す" else "リノート", colors, entry.operationStatus[EntryOperation.Repost] != EntryOperationStatus.Sending) {
                action(EntryOperation.Repost, "")
            }
            if (entry.canQuoteRenote) SheetAction(UiIcon.QuoteRenote, "引用リノート", colors) { action(EntryOperation.QuoteRenote, "") }
            SheetAction(UiIcon.Copy, "本文をコピー", colors) {
                scope.launch {
                    clipboard.setClipEntry(ClipEntry(ClipData.newPlainText(null, entry.body)))
                    dismiss()
                }
            }
        }
    }
}

@Composable
private fun SheetAction(icon: UiIcon, title: String, colors: AndroidColors, enabled: Boolean = true, click: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
            .clickable(enabled = enabled, role = Role.Button, onClick = click).padding(horizontal = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        UiGlyph(icon, colors.foreground, iconSize = AndroidControlSize.inlineIcon)
        Text(title, color = colors.foreground, fontWeight = AndroidWeights.emphasis)
    }
}


@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun RenoteSheet(entry: TimelineItem, colors: AndroidColors, dismiss: () -> Unit, choose: (EntryOperation) -> Unit) {
    ModalBottomSheet(onDismissRequest = dismiss, containerColor = colors.background) {
        Column(Modifier.fillMaxWidth().padding(bottom = 12.dp)) {
            SheetAction(UiIcon.Repost, if (entry.reposted) "リノートを取り消す" else "リノート", colors) { choose(EntryOperation.Repost) }
            SheetAction(UiIcon.QuoteRenote, "引用リノート", colors) { choose(EntryOperation.QuoteRenote) }
        }
    }
}
