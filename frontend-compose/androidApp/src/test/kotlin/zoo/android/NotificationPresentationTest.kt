package zoo.android

import android.app.Notification
import android.graphics.Bitmap
import android.os.Build
import android.os.Bundle
import kotlin.test.*
import kotlinx.serialization.json.JsonArray
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import zoo.*

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 28, 35], manifest = Config.NONE)
class NotificationPresentationTest {
    private fun row(kind: String = "mention", channel: String = "general", dm: Boolean = false) = obj(
        "id" to "notice", "actor_id" to "alice", "channel_id" to channel,
        "notification_type" to kind, "created_at" to "2026-11-16T01:00:00Z",
        "display_data" to obj(
            "actor" to obj("display_name" to "Alice", "avatar_url" to "https://media.test/alice.png"),
            "channel" to obj("channel_name" to channel, "is_dm" to dm),
            "message" to obj("content" to "長いメッセージ\n二行目"),
            "reactions" to JsonArray(listOf(
                obj("user_id" to "someone-else", "emoji" to "❌"),
                obj("user_id" to "alice", "emoji" to "👍"),
            )),
        ),
    )

    @Test fun messageStylePreservesSenderTimestampBodyAndDmIdentity() {
        val context = RuntimeEnvironment.getApplication()
        // MessagingStyle infers group conversations from titles for target SDK < 28.
        context.applicationInfo.targetSdkVersion = 35
        for (dm in listOf(false, true)) {
            val row = row(if (dm) "dm" else "mention", dm = dm)
            val avatar = Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888)
            val body = notificationBody(row) { _, _ -> error("Message should use its content") }
            val style = notificationStyle(row, body, "Me", avatar) as Notification.MessagingStyle
            val notification = Notification.Builder(context, "test")
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setStyle(style).build()
            assertEquals(style.conversationTitle,
                notification.extras.getCharSequence(Notification.EXTRA_CONVERSATION_TITLE))
            if (dm && Build.VERSION.SDK_INT < 28) assertNull(style.conversationTitle)
            else assertEquals("general", style.conversationTitle.toString())
            val message = style.messages.single()
            val serialized = notification.extras.getParcelableArray(Notification.EXTRA_MESSAGES)?.single() as Bundle
            assertEquals(message.text, serialized.getCharSequence("text"))
            assertEquals(message.timestamp, serialized.getLong("time"))
            assertEquals("長いメッセージ\n二行目", message.text.toString())
            assertEquals(timestampMillis(row.text("created_at")), message.timestamp)
            assertEquals("Alice", message.sender.toString())
            if (Build.VERSION.SDK_INT >= 28) {
                assertEquals(!dm, style.isGroupConversation)
                assertEquals("alice", message.senderPerson?.key)
                assertNotNull(message.senderPerson?.icon)
            }
        }
    }

    @Test fun missingAvatarDoesNotPreventMessageNotification() {
        val style = notificationStyle(row(), "body", "Me", null) as Notification.MessagingStyle
        assertEquals("body", style.messages.single().text.toString())
        if (Build.VERSION.SDK_INT >= 28) assertNull(style.messages.single().senderPerson?.icon)
    }

    @Test fun reactionsDescribeTheEventAndSelectTheActorsEmoji() {
        val row = row("reaction")
        val body = notificationBody(row) { key, actor -> "$key:$actor" }
        assertEquals("👍 notifications.reaction:Alice\n長いメッセージ\n二行目", body)
        assertIs<Notification.BigTextStyle>(notificationStyle(row, body, "Me", null))
    }

    @Test fun deletedMessageAndMissingActorKeepAFallback() {
        val row = obj("notification_type" to "mention", "display_data" to obj())
        assertEquals("notifications.mention:Zoo", notificationBody(row) { key, actor -> "$key:$actor" })
        assertEquals("", notificationAvatar(row))
    }

    @Test fun groupingCombinesMentionsAndRepliesButSeparatesChannelsAndReactions() {
        assertNotEquals(notificationGroup(row("mention")), notificationGroup(row("reply")))
        assertNotEquals(notificationGroup(row("channel_message")), notificationGroup(row("mention")))
        assertNotEquals(notificationGroup(row()), notificationGroup(row(channel = "other")))
        assertNotEquals(notificationGroup(row()), notificationGroup(row("reaction")))
        assertNotEquals(notificationGroup(row()), notificationGroup(row("dm")))
    }
}
