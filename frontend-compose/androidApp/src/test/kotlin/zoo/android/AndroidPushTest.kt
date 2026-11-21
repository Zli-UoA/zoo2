package zoo.android

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.Context
import java.util.UUID
import kotlin.test.*
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import zoo.android.phone.DM_PUSH_ANSWER_ACTION
import zoo.android.phone.DM_PUSH_OPEN_ACTION
import zoo.android.phone.DmCallPushAnswer
import zoo.android.phone.IncomingCallActivity

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class AndroidPushTest {
    private val context: Context get() = RuntimeEnvironment.getApplication()
    private val manager get() = context.getSystemService(NotificationManager::class.java)
    private val user = UUID.randomUUID().toString()
    private val channel = UUID.randomUUID().toString()
    private val message = UUID.randomUUID().toString()
    private val id = UUID.randomUUID().toString()

    @BeforeTest fun prepare() {
        context.getSharedPreferences("push-binding", Context.MODE_PRIVATE).edit().clear().commit()
        context.getSharedPreferences("push-subscription-cleanup", Context.MODE_PRIVATE).edit().clear().commit()
        manager.cancelAll()
        shadowOf(RuntimeEnvironment.getApplication()).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
    }

    private fun data(binding: String) = mapOf(
        "binding" to binding, "userId" to user, "title" to "投稿者", "body" to "新しいメッセージ",
        "url" to "/channels/$channel?message=$message", "tag" to "zoo-$id", "notificationType" to "dm",
    )

    @Test fun coldStartReceivesAndRepeatedDeliveryReplacesTheSameNotification() {
        val binding = PushBinding(context).bind("https://zoo.example", user)
        // No Activity, Session, coroutine scope or in-memory notification controller is present.
        repeat(2) { showPushNotification(context, data(binding)) }
        val notifications = manager.activeNotifications
        assertEquals(1, notifications.size)
        assertEquals(notificationTag(user, id), notifications.single().tag)
        assertEquals("新しいメッセージ", notifications.single().notification.extras.getString(Notification.EXTRA_TEXT))
        assertEquals(Notification.VISIBILITY_PRIVATE, notifications.single().notification.visibility)
        assertNotNull(notifications.single().notification.contentIntent)
    }

    @Test fun accountSwitchAndLogoutRejectDelayedDeliveries() {
        val store = PushBinding(context)
        val old = store.bind("https://zoo.example", user)
        val other = store.bind("https://other.example", user)
        showPushNotification(context, data(old))
        assertTrue(manager.activeNotifications.isEmpty())
        store.clear(old) // Disposing an old session must not clear the new one.
        assertTrue(store.matches(other, user))
        store.clear(other)
        showPushNotification(context, data(other))
        assertTrue(manager.activeNotifications.isEmpty())
    }

    @Test fun deniedPermissionWrongRecipientAndUnsupportedDestinationsDoNotNotify() {
        val binding = PushBinding(context).bind("https://zoo.example", user)
        showPushNotification(context, data(binding) + ("userId" to "someone-else"))
        showPushNotification(context, data(binding) + ("url" to "https://evil.example/"))
        showPushNotification(context, data(binding) + ("notificationType" to "call_invite"))
        shadowOf(RuntimeEnvironment.getApplication()).denyPermissions(Manifest.permission.POST_NOTIFICATIONS)
        showPushNotification(context, data(binding))
        assertTrue(manager.activeNotifications.isEmpty())
    }

    @Test fun optedInDmCallPushRingsOnColdStartAndAnEndPreventsReorderedDelivery() {
        val store = PushBinding(context)
        val binding = store.bind("https://zoo.example", user)
        val payload = data(binding) + mapOf(
            "notificationType" to "call_invite", "inviteId" to id, "title" to "山田 葵",
            "expiresAt" to (System.currentTimeMillis() / 1000L + 25).toString(),
        )
        showPushNotification(context, payload)
        assertTrue(manager.activeNotifications.isEmpty())

        store.setDmCallsEnabled(user, true)
        repeat(2) { showPushNotification(context, payload) }
        val notification = manager.activeNotifications.single().notification
        assertEquals(Notification.CATEGORY_CALL, notification.category)
        assertTrue(notification.flags and Notification.FLAG_ONGOING_EVENT != 0)
        val channel = manager.getNotificationChannel(notification.channelId)
        assertEquals(NotificationManager.IMPORTANCE_HIGH, channel.importance)
        assertTrue(channel.shouldVibrate())
        val fullScreen = assertNotNull(notification.fullScreenIntent)
        val incomingScreen = shadowOf(fullScreen).savedIntent
        assertEquals(IncomingCallActivity::class.java.name, incomingScreen.component?.className)
        assertEquals("山田 葵", incomingScreen.getStringExtra("callerName"))
        val intent = notification.actions.map { shadowOf(it.actionIntent).savedIntent }
            .single { it.action == DM_PUSH_ANSWER_ACTION }
        val open = shadowOf(notification.contentIntent).savedIntent
        assertEquals(DM_PUSH_OPEN_ACTION, open.action)
        assertEquals(id, DmCallPushAnswer.fromIntent(open)?.id)
        assertEquals(DM_PUSH_ANSWER_ACTION, intent.action)
        assertEquals(id, DmCallPushAnswer.fromIntent(intent)?.id)
        assertEquals(binding, DmCallPushAnswer.fromIntent(intent)?.binding)

        showPushNotification(context, payload + ("notificationType" to "call_invite_end"))
        showPushNotification(context, payload)
        assertTrue(manager.activeNotifications.isEmpty())
        showPushNotification(context, payload + mapOf("inviteId" to UUID.randomUUID().toString(), "expiresAt" to "1"))
        assertTrue(manager.activeNotifications.isEmpty())
    }

    @Test fun disablingDmCallingClearsRingingNotificationAndRejectsNewOrStaleBinding() {
        val store = PushBinding(context)
        val binding = store.bind("https://zoo.example", user)
        store.setDmCallsEnabled(user, true)
        val payload = data(binding) + mapOf(
            "notificationType" to "call_invite", "inviteId" to id,
            "expiresAt" to (System.currentTimeMillis() / 1000L + 25).toString(),
        )
        showPushNotification(context, payload + ("binding" to UUID.randomUUID().toString()))
        assertTrue(manager.activeNotifications.isEmpty())
        showPushNotification(context, payload)
        assertEquals(1, manager.activeNotifications.size)
        store.setDmCallsEnabled(user, false)
        assertTrue(manager.activeNotifications.isEmpty())
        showPushNotification(context, payload)
        assertTrue(manager.activeNotifications.isEmpty())
    }

    @Test fun lockScreenFinishesIfRingEndedBeforeItWasCreated() {
        val action = PendingIntent.getBroadcast(context, 0, Intent("zoo.test.action"), PendingIntent.FLAG_IMMUTABLE)
        val screen = Intent(context, IncomingCallActivity::class.java)
            .putExtra("answer", action).putExtra("dismiss", action)
        val activity = Robolectric.buildActivity(IncomingCallActivity::class.java, screen).create().get()
        assertTrue(activity.isFinishing)
    }

    @Test fun notificationTapUsesOnlyAnInternalMessageDestination() {
        assertEquals("/channels/$channel?message=$message", pushDestination("/channels/$channel?message=$message"))
        for (invalid in listOf(null, "//evil.example/", "/channels/$channel?message=$message&url=evil", "/settings")) {
            assertNull(pushDestination(invalid))
        }
    }
}
