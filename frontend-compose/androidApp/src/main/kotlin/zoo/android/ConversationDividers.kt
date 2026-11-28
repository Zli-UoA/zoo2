package zoo.android

import zoo.ChannelReadPosition
import zoo.PageState
import zoo.android.ui.TimelineItem
import zoo.timestampMillis

// Same adjacent-message gap as the Web channel feed.
private const val MESSAGE_GROUP_GAP_MS = 5 * 60 * 1000L

/** Preserve message keys and scroll indexes when a divider appears or older history is prepended. */
internal fun withConversationDividers(
    entries: List<TimelineItem>,
    unreadId: String?,
    day: (String) -> String,
    dateLabel: (String) -> String,
    unreadLabel: String,
): List<TimelineItem> {
    var previousDay: String? = null
    var previousAuthor: String? = null
    var previousTime: Long? = null
    return entries.map { entry ->
        val timestamp = entry.createdAt
        val currentDay = timestamp.takeIf { it.isNotBlank() }?.let(day)
        val sameDay = currentDay != null && currentDay == previousDay
        val author = entry.authorId.takeIf { it.isNotBlank() }
        val time = timestampMillis(timestamp)
        val gap = time?.let { current -> previousTime?.let { current - it } }
        val unreadDivider = unreadLabel.takeIf { entry.id == unreadId }
        val continuesGroup = sameDay && unreadDivider == null && entry.divider == null &&
            author != null && author == previousAuthor && gap != null && gap in 0..MESSAGE_GROUP_GAP_MS
        previousDay = currentDay
        previousAuthor = author
        previousTime = time
        entry.copy(
            dateDivider = if (currentDay != null && !sameDay) dateLabel(timestamp) else null,
            unreadDivider = unreadDivider,
            continuesGroup = continuesGroup,
        )
    }
}

internal fun unreadHistoryOutsideWindow(page: PageState?, position: ChannelReadPosition?): Boolean {
    if (position?.hasUnread != true) return false
    if (page?.loaded != true) return true
    if (!page.hasOlder) return false
    val oldest = page.oldest?.first?.let(::timestampMillis) ?: return true
    return oldest > position.timestamp
}
