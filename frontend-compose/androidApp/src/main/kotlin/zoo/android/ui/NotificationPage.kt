package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay

internal data class InboxActor(val id: String, val name: String, val avatar: String? = null)

internal data class InboxReactionEmoji(val emoji: String, val image: String? = null)
internal data class InboxReactionContext(val id: String, val actorIds: Set<String>)

internal data class InboxItem(
    val id: String,
    val actors: List<InboxActor>,
    val summary: String,
    val detail: String?,
    val time: String,
    val unread: Boolean,
    val target: String? = null,
    val followActor: String? = null, val followActivity: String? = null,
    val kind: String = "", val unreadIds: List<String> = emptyList(), val destination: String? = null,
    val reactions: List<InboxReactionEmoji> = emptyList(),
    val reactionContext: InboxReactionContext? = null,
)

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun NotificationPage(
    items: List<InboxItem>,
    foreground: Boolean,
    colors: AndroidColors,
    nextCursor: String?,
    loadAvatar: suspend (String) -> ImageBitmap?,
    loadReactions: suspend (InboxReactionContext) -> List<InboxReactionEmoji>,
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
            Row(
                Modifier.fillMaxWidth()
                    .then(if (target != null) Modifier.clickable { onOpen(target) } else Modifier)
                    .padding(vertical = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Box(Modifier.size(42.dp)) {
                    val actor = item.actors.firstOrNull()
                    if (actor != null) InboxAvatar(actor, 40.dp, colors, loadAvatar)
                    Box(
                        Modifier.align(Alignment.BottomEnd).size(20.dp)
                            .background(colors.background, CircleShape).padding(2.dp)
                            .background(colors.quietSurface, CircleShape),
                        contentAlignment = Alignment.Center,
                    ) { UiGlyph(item.kind.activityIcon(), colors.foreground, iconSize = 12.dp) }
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(5.dp)) {
                    Column {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                            val author = item.actors.firstOrNull()?.name.orEmpty()
                            Text(if (item.actors.size > 1) "$author ほか" else author, Modifier.weight(1f),
                                color = colors.foreground, fontWeight = AndroidWeights.userName, maxLines = 1,
                                overflow = TextOverflow.Ellipsis)
                            Text(item.time, color = colors.secondary, fontSize = 12.sp)
                            if (item.unread) Box(
                                Modifier.size(8.dp).background(colors.accent, CircleShape).border(1.dp, colors.secondary, CircleShape)
                                    .semantics { contentDescription = "未読" },
                            )
                        }
                        val channelReactions by key(item.reactionContext) {
                            produceState(emptyList<InboxReactionEmoji>(), foreground && item.id in visible) {
                                if (foreground && item.id in visible) {
                                    value = item.reactionContext?.let { loadReactions(it) }.orEmpty()
                                }
                            }
                        }
                        FlowRow(
                            horizontalArrangement = Arrangement.spacedBy(6.dp),
                            itemVerticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(item.summary, color = colors.secondary, fontSize = 13.sp)
                            (item.reactions + channelReactions).distinct().forEach { reaction ->
                                key(reaction) { InboxReaction(reaction, colors, loadAvatar) }
                            }
                        }
                    }
                    if (item.actors.size > 1) {
                        LazyRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                            items(item.actors.drop(1), key = { it.id }) { actor ->
                                InboxAvatar(actor, 24.dp, colors, loadAvatar)
                            }
                        }
                    }
                    item.detail?.let {
                        Text(formatEntry(it), color = colors.foreground, fontSize = 14.sp,
                            maxLines = 3, overflow = TextOverflow.Ellipsis)
                    }
                    item.destination?.let {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                            UiGlyph(if (it.startsWith("#")) UiIcon.Channels else UiIcon.Public, colors.secondary, iconSize = 14.dp)
                            Text(it, color = colors.secondary, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                    if (item.followActor != null && item.followActivity != null) {
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            TextAction("承認", colors, icon = UiIcon.Check) { respond(item.followActor, item.followActivity, true) }
                            TextAction("拒否", colors, icon = UiIcon.Close, style = TextActionStyle.Neutral) { respond(item.followActor, item.followActivity, false) }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun InboxReaction(
    reaction: InboxReactionEmoji,
    colors: AndroidColors,
    loadImage: suspend (String) -> ImageBitmap?,
) {
    val bitmap by produceState<ImageBitmap?>(null, reaction.image) {
        value = reaction.image?.let { loadImage(it) }
    }
    val image = bitmap
    if (image != null) Image(image, reaction.emoji, Modifier.size(18.dp), contentScale = ContentScale.Fit)
    else Text(reaction.emoji, color = colors.foreground, fontSize = 14.sp)
}

@Composable
private fun InboxAvatar(
    actor: InboxActor,
    size: Dp,
    colors: AndroidColors,
    loadAvatar: suspend (String) -> ImageBitmap?,
) {
    val bitmap by key(actor.avatar) {
        produceState<ImageBitmap?>(null) { value = actor.avatar?.let { loadAvatar(it) } }
    }
    Box(Modifier.size(size).clip(CircleShape).background(colors.inputSurface)
        .semantics { contentDescription = actor.name }, contentAlignment = Alignment.Center) {
        val loaded = bitmap
        if (loaded != null) Image(loaded, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        else Text(actor.name.take(1), color = colors.foreground, fontSize = 13.sp, fontWeight = AndroidWeights.userName)
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
