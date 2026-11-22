package zoo.android

import zoo.NotificationInbox
import zoo.NotificationReactionContext
import zoo.android.ui.AndroidScreen
import zoo.android.ui.AndroidUiAction
import zoo.android.ui.InboxItem
import zoo.android.ui.InboxActor
import zoo.android.ui.InboxReactionContext
import zoo.android.ui.InboxReactionEmoji

internal class NotificationInboxBinding(private val inbox: NotificationInbox) {
    fun screen(): AndroidScreen.Notifications = AndroidScreen.Notifications(
        inbox.snapshot().map { InboxItem(it.id, it.actors.map { actor -> InboxActor(actor.id, actor.name, actor.avatar) },
            it.summary, it.snippet, it.time, it.unread, it.target,
            it.followActor, it.followActivity, it.kind, it.unreadIds, it.destination,
            it.reactions.map { reaction -> InboxReactionEmoji(reaction.emoji, reaction.image) },
            it.reactionContext?.let { context -> InboxReactionContext(context.id, context.actorIds) }) },
        nextCursor = inbox.nextCursor,
        loadFailed = inbox.loadFailed,
    )

    suspend fun loadReactions(context: InboxReactionContext): List<InboxReactionEmoji> =
        inbox.loadReactions(NotificationReactionContext(context.id, context.actorIds))
            .map { InboxReactionEmoji(it.emoji, it.image) }

    fun dispatch(action: AndroidUiAction.ViewedNotification) {
        inbox.viewed(action.id)
    }
}
