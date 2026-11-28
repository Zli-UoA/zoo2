package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties

internal sealed interface DocumentViewerState {
    data object Loading : DocumentViewerState
    data object Failed : DocumentViewerState
    data class Ready(val pages: Int) : DocumentViewerState
}

/**
 * 添付の PDF の全画面表示（iOS の `PDFPreviewView`）。ページを縦に並べてスクロールし、下に今のページを出す。
 * ヘッダーの戻るで閉じ、保存ボタンで端末の「ダウンロード」に保存する。ページの描画は呼び出し側が行う。
 */
@Composable
internal fun DocumentViewer(
    name: String,
    dark: Boolean,
    state: DocumentViewerState,
    ratio: suspend (Int) -> Float?,
    page: suspend (index: Int, width: Int) -> ImageBitmap?,
    save: () -> Unit,
    retry: () -> Unit,
    dismiss: () -> Unit,
) {
    Dialog(onDismissRequest = dismiss, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        AndroidShell(null, dark, {}, allowRail = false) { colors ->
            PageHeader(name, colors, onBack = dismiss) {
                UiIconButton(UiIcon.Download, androidLabel("attachments.downloadShort", "ダウンロード"), colors.foreground, save,
                    modifier = Modifier.testTag("document-save"))
            }
            Box(Modifier.fillMaxSize().background(colors.quietSurface), contentAlignment = Alignment.Center) {
                when (state) {
                    DocumentViewerState.Failed -> Column(horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text(androidLabel("attachments.loadError", "プレビューを表示できません"), color = colors.foreground)
                        TextAction(androidLabel("attachments.retry", "再試行"), colors, icon = UiIcon.Refresh, click = retry)
                    }
                    DocumentViewerState.Loading -> CircularProgressIndicator(color = colors.accent)
                    is DocumentViewerState.Ready -> DocumentPages(name, state.pages, ratio, page, colors)
                }
            }
        }
    }
}

@Composable
private fun DocumentPages(
    name: String, count: Int, ratio: suspend (Int) -> Float?, page: suspend (Int, Int) -> ImageBitmap?, colors: AndroidColors,
) {
    val list = rememberLazyListState()
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val width = constraints.maxWidth
        LazyColumn(Modifier.fillMaxSize().testTag("document-pages"), state = list,
            contentPadding = PaddingValues(12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            items(count) { index ->
                val shape by produceState(A4_RATIO, index) { value = ratio(index) ?: A4_RATIO }
                val image by produceState<ImageBitmap?>(null, index, width) { value = page(index, width) }
                val label = "$name ${index + 1} / $count"
                Box(Modifier.fillMaxWidth().aspectRatio(1f / shape).background(Color.White)
                    .semantics { contentDescription = label }) {
                    image?.let { Image(it, null, Modifier.fillMaxSize(), contentScale = ContentScale.Fit) }
                }
            }
        }
        Text("${list.firstVisibleItemIndex + 1} / $count",
            Modifier.align(Alignment.BottomCenter).padding(16.dp)
                .background(colors.foreground.copy(alpha = 0.8f), CircleShape)
                .padding(horizontal = 12.dp, vertical = 4.dp).testTag("document-page-number"),
            color = colors.background, fontSize = 13.sp)
    }
}

private const val A4_RATIO = 1.414f
