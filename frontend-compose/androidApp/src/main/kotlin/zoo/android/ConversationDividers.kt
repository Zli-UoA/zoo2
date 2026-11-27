package zoo.android

import zoo.ChannelReadPosition
import zoo.PageState
import zoo.Row
import zoo.android.ui.TimelineItem
import zoo.text
import zoo.timestampMillis

/** Preserve message keys and scroll indexes when a divider appears or older history is prepended. */
internal fun withConversationDividers(
    entries: List<TimelineItem>,
    rowsById: Map<String, Row>,
    unreadId: String?,
    day: (String) -> String,
    dateLabel: (String) -> String,
    unreadLabel: String,
): List<TimelineItem> {
    var previousDay: String? = null
    return entries.map { entry ->
        val timestamp = rowsById[entry.id]?.text("created_at").orEmpty()
        val currentDay = timestamp.takeIf { it.isNotBlank() }?.let(day)
        val label = if (currentDay != null && currentDay != previousDay) dateLabel(timestamp) else null
        previousDay = currentDay
        entry.copy(dateDivider = label, unreadDivider = unreadLabel.takeIf { entry.id == unreadId })
    }
}

internal fun unreadHistoryOutsideWindow(page: PageState?, position: ChannelReadPosition?): Boolean {
    if (position?.hasUnread != true) return false
    if (page?.loaded != true) return true
    if (!page.hasOlder) return false
    val oldest = page.oldest?.first?.let(::timestampMillis) ?: return true
    return oldest > position.timestamp
}
