package zoo.android

import android.Manifest
import android.app.Activity
import android.app.Notification
import android.app.NotificationManager
import android.content.Intent
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import zoo.*

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 35], manifest = Config.NONE)
class AndroidNotificationsTest {
    private class Fixture(scope: CoroutineScope) {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val manager = activity.getSystemService(NotificationManager::class.java)
        val requested = mutableListOf<String>()
        val image = CompletableDeferred<ByteArray>()
        val platform = object : Platform {
            override suspend fun headers() = emptyMap<String, String>()
            override suspend fun readCache(key: String): String? = null
            override suspend fun writeCache(key: String, value: String) {}
            override fun uuid() = "test"
            override fun now() = "2026-11-16T00:00:00Z"
            override fun navigate(path: String) {}
            override fun action(name: String) {}
            override fun translate(key: String, args: Map<String, String>) = key
            override fun time(timestamp: String) = timestamp
            override fun day(timestamp: String) = timestamp
            override suspend fun image(url: String): ByteArray {
                requested += url
                return image.await()
            }
        }
        var apiCalls = 0
        val http = HttpClient(MockEngine { apiCalls++; error("Network unavailable") })
        val app = Session(Repository(zoo.Config("https://zoo.test", "/api/shapes", "me", false),
            platform, http, scope, synchronize = false), scope, Environment(visible = false))
        val notifications: AndroidNotifications

        init {
            shadowOf(activity.application).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
            notifications = AndroidNotifications(activity, app)
            notifications.update(emptyList())
        }

        fun rows() = manager.activeNotifications.toList()
        fun summary() = rows().single { it.notification.flags and Notification.FLAG_GROUP_SUMMARY != 0 }
        fun close() { notifications.close(); app.close(); http.close(); activity.finish() }
    }

    private fun row(id: String, channel: String = "general", avatar: Boolean = false) = obj(
        "id" to id, "channel_id" to channel, "actor_id" to "alice", "notification_type" to "mention",
        "created_at" to "2026-11-16T00:00:00Z", "message_id" to "message-$id",
        "display_data" to obj("actor" to obj("display_name" to "Alice",
            "avatar_url" to if (avatar) "https://media.test/alice.png" else ""),
            "channel" to obj("channel_name" to channel), "message" to obj("content" to "body-$id")),
    )

    @Test fun summariesAppearForTwoNotificationsAndDisappearWhenOneIsRead() = runTest {
        val f = Fixture(backgroundScope)
        try {
            val first = row("1")
            val second = row("2")
            f.notifications.update(listOf(first))
            assertEquals(1, f.rows().size)
            f.notifications.update(listOf(second, first))
            assertEquals(3, f.rows().size)
            val summary = f.summary().notification
            assertEquals(2, summary.number)
            assertEquals(Notification.VISIBILITY_PRIVATE, summary.visibility)
            assertEquals(Notification.GROUP_ALERT_CHILDREN, summary.groupAlertBehavior)
            assertEquals(listOf("Alice: body-1", "Alice: body-2"),
                summary.extras.getCharSequenceArray(Notification.EXTRA_TEXT_LINES)?.map { it.toString() })
            f.notifications.update(listOf(first))
            assertEquals(1, f.rows().size)
            f.notifications.update(emptyList())
            assertTrue(f.rows().isEmpty())
            f.notifications.update(listOf(first, second))
            assertTrue(f.rows().isEmpty(), "Old IDs must not be reposted")
        } finally { f.close() }
    }

    @Test fun dismissingAGroupDoesNotDismissAnotherChannelOrMarkMessagesRead() = runTest {
        val f = Fixture(backgroundScope)
        try {
            f.notifications.update(listOf(row("1"), row("2"), row("3", "other")))
            val dismiss = shadowOf(f.summary().notification.deleteIntent).savedIntent
            f.notifications.receive(Intent(dismiss).putExtra("session", "stale-session"))
            assertEquals(4, f.rows().size)
            f.notifications.receive(dismiss)
            assertEquals(1, f.rows().size)
            assertTrue(f.rows().single().tag.endsWith(":3"))
            assertEquals(0, f.apiCalls)
            f.notifications.close()
            assertTrue(f.rows().isEmpty())
        } finally { f.close() }
    }

    @Test fun sharedSlowAvatarsDoNotDelayNotificationsOrReviveThemAfterLogout() = runTest {
        val f = Fixture(backgroundScope)
        try {
            f.notifications.update(listOf(row("1", avatar = true), row("2", avatar = true)))
            assertEquals(3, f.rows().size, "Post text before waiting for an image")
            runCurrent()
            assertEquals(listOf("https://media.test/alice.png"), f.requested)
            f.notifications.close()
            f.image.complete(byteArrayOf())
            runCurrent()
            assertTrue(f.rows().isEmpty())
            f.notifications.update(listOf(row("3")))
            assertTrue(f.rows().isEmpty(), "A closed session cannot post notifications")
        } finally { f.close() }
    }

    @Test fun groupOverviewIsBoundedAndIndividualActionsRemainAvailable() = runTest {
        val f = Fixture(backgroundScope)
        try {
            f.notifications.update((1..7).map { row(it.toString()) })
            assertEquals(8, f.rows().size)
            assertEquals(7, f.summary().notification.number)
            assertEquals(5, f.summary().notification.extras
                .getCharSequenceArray(Notification.EXTRA_TEXT_LINES)?.size)
            f.rows().filter { it.notification.flags and Notification.FLAG_GROUP_SUMMARY == 0 }
                .forEach { assertEquals(1, it.notification.actions.size) }
        } finally { f.close() }
    }

    @Test fun synchronizedEditsDoNotEraseAnActionError() = runTest {
        val f = Fixture(backgroundScope)
        try {
            val original = row("1")
            f.notifications.update(listOf(original))
            val read = shadowOf(f.rows().single().notification.actions.single().actionIntent).savedIntent
            f.notifications.receive(read)
            assertEquals(1, f.apiCalls)
            assertEquals("notifications.readError", f.rows().single().notification.extras
                .getCharSequence(Notification.EXTRA_TEXT).toString())
            f.notifications.update(listOf(obj(*original.map { it.key to it.value }.toTypedArray(),
                "display_data" to obj("actor" to obj("display_name" to "Alice Updated"),
                    "message" to obj("content" to "edited")))))
            assertEquals("notifications.readError", f.rows().single().notification.extras
                .getCharSequence(Notification.EXTRA_TEXT).toString())
        } finally { f.close() }
    }
}
