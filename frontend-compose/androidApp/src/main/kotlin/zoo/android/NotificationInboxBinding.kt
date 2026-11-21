package zoo.android

import zoo.NotificationInbox
import zoo.android.ui.AndroidScreen
import zoo.android.ui.AndroidUiAction
import zoo.android.ui.InboxItem

internal class NotificationInboxBinding(private val inbox: NotificationInbox) {
    fun screen(): AndroidScreen.Notifications = AndroidScreen.Notifications(
        inbox.snapshot().map { InboxItem(it.id, it.author, it.summary, it.snippet, it.time, it.unread, it.target) },
    )

    fun dispatch(action: AndroidUiAction.ViewedNotification) {
        inbox.viewed(action.id)
    }
}
