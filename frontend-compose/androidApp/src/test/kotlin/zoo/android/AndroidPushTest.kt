package zoo.android

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.graphics.Bitmap
import kotlinx.coroutines.test.runTest
import java.util.UUID
import kotlin.test.*
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

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

    @Test fun publicImageUpgradesExistingPushToBigPicture() = runTest {
        val binding = PushBinding(context).bind("https://zoo.example", user)
        val attachment = UUID.randomUUID().toString()
        val posted = assertNotNull(showPushNotification(context, data(binding) + mapOf(
            "notificationType" to "mention", "actor" to "投稿者", "channelName" to "開発",
            "avatarUrl" to "https://zoo.example/avatar.png", "imageId" to attachment,
        )))
        assertEquals("https://zoo.example/api/home/images/$attachment", posted.imageUrl)
        assertEquals("android.app.Notification\$MessagingStyle",
            manager.activeNotifications.single().notification.extras.getString(Notification.EXTRA_TEMPLATE))
        val requested = mutableSetOf<String>()
        enrichPushNotification(context, posted) { url, _ ->
            requested += url
            Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888)
        }
        val displayed = manager.activeNotifications.single().notification
        assertEquals("android.app.Notification\$BigPictureStyle", displayed.extras.getString(Notification.EXTRA_TEMPLATE))
        assertNotNull(displayed.getLargeIcon())
        assertEquals(setOf(posted.avatarUrl, posted.imageUrl), requested)
        assertEquals(Notification.VISIBILITY_PRIVATE, displayed.visibility)
    }

    @Test fun dismissedPushIsNotRestoredByLateMedia() = runTest {
        val binding = PushBinding(context).bind("https://zoo.example", user)
        val posted = assertNotNull(showPushNotification(context, data(binding) +
            ("avatarUrl" to "https://zoo.example/avatar.png")))
        enrichPushNotification(context, posted) { _, _ ->
            manager.cancel(posted.tag, 0)
            Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888)
        }
        assertTrue(manager.activeNotifications.isEmpty())
    }

    @Test fun blocksExternalAvatarAndDoesNotRequestAttachmentsForDm() {
        val binding = PushBinding(context).bind("https://zoo.example", user)
        val posted = assertNotNull(showPushNotification(context, data(binding) + mapOf(
            "avatarUrl" to "https://elsewhere.example/avatar.png", "imageId" to UUID.randomUUID().toString(),
        )))
        assertNull(posted.avatarUrl)
        assertNull(posted.imageUrl)
    }

    @Test fun notificationTapUsesOnlyAnInternalMessageDestination() {
        assertEquals("/channels/$channel?message=$message", pushDestination("/channels/$channel?message=$message"))
        for (invalid in listOf(null, "//evil.example/", "/channels/$channel?message=$message&url=evil", "/settings")) {
            assertNull(pushDestination(invalid))
        }
    }
}
