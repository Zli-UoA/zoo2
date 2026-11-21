package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
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
    val target: String? = null,
    val followActor: String? = null, val followActivity: String? = null,
    val kinds: List<String> = emptyList(), val avatar: String? = null, val unreadIds: List<String> = emptyList(),
)

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun NotificationPage(
    items: List<InboxItem>,
    foreground: Boolean,
    colors: AndroidColors,
    nextCursor: String?,
    loadAvatar: suspend (String) -> ImageBitmap?,
    onViewed: (String) -> Unit,
    onOpen: (String) -> Unit,
    loadMore: () -> Unit,
    respond: (String, String, Boolean) -> Unit,
) {
    PageHeader("通知", colors, paintBackground = false)
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
    val nextPage by rememberUpdatedState(loadMore)
    val lastVisible = items.lastOrNull()?.id?.takeIf { it in visible }
    LaunchedEffect(lastVisible, foreground, nextCursor) {
        if (foreground && lastVisible != null && nextCursor != null) nextPage()
    }
    val viewed by rememberUpdatedState(onViewed)
    LazyColumn(state = scroll, modifier = Modifier.fillMaxSize(), contentPadding = PaddingValues(horizontal = 16.dp)) {
        items(items, key = { it.id }) { item ->
            val shouldRead = foreground && item.unread && item.id in visible
            LaunchedEffect(item.id, shouldRead, item.unreadIds) {
                if (shouldRead) {
                    delay(500)
                    viewed(item.id)
                }
            }
            val target = item.target
            Row(Modifier.fillMaxWidth()
                .then(if (target != null) Modifier.clickable { onOpen(target) } else Modifier)
                .padding(vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                val avatar by key(item.avatar) {
                    produceState<ImageBitmap?>(null, foreground) {
                        value = if (foreground) item.avatar?.let { loadAvatar(it) } else null
                    }
                }
                Box(Modifier.size(40.dp).clip(CircleShape).background(colors.inputSurface),
                    contentAlignment = Alignment.Center) {
                    val bitmap = avatar
                    if (bitmap != null) Image(bitmap, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
                    else UiGlyph(item.kinds.firstOrNull().activityIcon(), colors.foreground, iconSize = 20.dp)
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(item.author, Modifier.weight(1f), color = colors.foreground, fontWeight = AndroidWeights.userName)
                        Text(item.time, color = colors.secondary, fontSize = 12.sp)
                        if (item.unread) Box(Modifier.size(8.dp).background(colors.accent, CircleShape)
                            .border(1.dp, colors.secondary, CircleShape).semantics { contentDescription = "未読" })
                    }
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                        if (avatar != null || item.kinds.size > 1) {
                            item.kinds.forEach { UiGlyph(it.activityIcon(), colors.secondary, iconSize = 15.dp) }
                        }
                        Text(item.summary, color = colors.foreground, fontSize = 15.sp)
                    }
                    item.detail?.let { Text(it, color = colors.secondary, fontSize = 13.sp) }
                    if (item.followActor != null && item.followActivity != null) {
                        androidx.compose.foundation.layout.FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            TextAction("承認", colors, icon = UiIcon.Check) { respond(item.followActor, item.followActivity, true) }
                            TextAction("拒否", colors, icon = UiIcon.Close, style = TextActionStyle.Neutral) { respond(item.followActor, item.followActivity, false) }
                        }
                    }
                }
            }
        }
    }
}

private fun String?.activityIcon(): UiIcon = when (this) {
    "like" -> UiIcon.Emoji
    "reply", "dm" -> UiIcon.Bubble
    "mention" -> UiIcon.AlternateEmail
    "repost" -> UiIcon.Repost
    "follow_request", "follow_accepted" -> UiIcon.UserAdd
    else -> UiIcon.Bell
}
