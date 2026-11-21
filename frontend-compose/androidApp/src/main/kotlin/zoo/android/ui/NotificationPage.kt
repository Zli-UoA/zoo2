package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay

internal data class InboxItem(
    val id: String,
    val author: String,
    val summary: String,
    val detail: String?,
    val time: String,
    val unread: Boolean,
)

@Composable
internal fun NotificationPage(
    items: List<InboxItem>,
    foreground: Boolean,
    colors: AndroidColors,
    onViewed: (String) -> Unit,
) {
    PageHeader("通知", colors)
    val scroll = rememberLazyListState()
    val visible by remember {
        derivedStateOf {
            val layout = scroll.layoutInfo
            layout.visibleItemsInfo.mapNotNull { row ->
                val overlap = minOf(row.offset + row.size, layout.viewportEndOffset) -
                    maxOf(row.offset, layout.viewportStartOffset)
                (row.key as? String)?.takeIf { overlap > 0 && overlap >= row.size * .1f }
            }.toSet()
        }
    }
    val viewed by rememberUpdatedState(onViewed)
    LazyColumn(state = scroll, modifier = Modifier.fillMaxSize(), contentPadding = PaddingValues(horizontal = 16.dp)) {
        items(items, key = { it.id }) { item ->
            val shouldRead = foreground && item.unread && item.id in visible
            LaunchedEffect(item.id, shouldRead) {
                if (shouldRead) {
                    delay(500)
                    viewed(item.id)
                }
            }
            Column(Modifier.fillMaxWidth().padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(item.author, Modifier.weight(1f), color = colors.foreground, fontWeight = AndroidWeights.userName)
                    Text(item.time, color = colors.secondary, fontSize = 12.sp)
                    if (item.unread) {
                        Text("未読", Modifier.background(colors.headerLight, CircleShape).padding(horizontal = 6.dp),
                            color = colors.foreground, fontSize = 12.sp)
                    }
                }
                Text(item.summary, color = colors.foreground, fontSize = 15.sp)
                item.detail?.let { Text(it, color = colors.secondary, fontSize = 13.sp) }
            }
        }
    }
}
