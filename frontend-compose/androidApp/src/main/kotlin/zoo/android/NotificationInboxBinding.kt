package zoo.android

import zoo.NotificationInbox
import zoo.android.ui.AndroidScreen
import zoo.android.ui.AndroidUiAction
import zoo.android.ui.InboxItem
import zoo.android.ui.InboxActor

internal class NotificationInboxBinding(private val inbox: NotificationInbox) {
    fun screen(): AndroidScreen.Notifications = AndroidScreen.Notifications(
        inbox.snapshot().map { InboxItem(it.id, it.actors.map { actor -> InboxActor(actor.id, actor.name, actor.avatar) },
            it.summary, it.snippet, it.time, it.unread, it.target,
            it.followActor, it.followActivity, it.kind, it.unreadIds, it.destination) },
        nextCursor = inbox.nextCursor,
    )

    fun dispatch(action: AndroidUiAction.ViewedNotification) {
        inbox.viewed(action.id)
    }
}
