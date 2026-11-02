package zoo

import kotlin.test.*

class UnifiedNotificationsTest {
    private fun local(id: String, time: String, read: Boolean = false): Row = obj("id" to id, "user_id" to "me", "actor_id" to "alice", "channel_id" to "channel", "message_id" to "message", "notification_type" to "reaction", "is_read" to read, "created_at" to time)
    private fun remote(id: String, time: String, actor: String = "https://peer/alice", read: Boolean = false): Row = obj("id" to id, "actor_id" to actor, "post_id" to "message", "kind" to "like", "is_read" to read, "created_at" to time)

    @Test fun interleavesSourcesByMicrosecondsAndKeepsOrderAfterReading() {
        val middle = local("local", "2026-10-24T01:00:00.000002Z")
        val rows = listOf(remote("new", "2026-10-24T01:00:00.000003Z"), remote("old", "2026-10-24T01:00:00.000001Z"))
        val feed = unifiedNotificationFeed(listOf(middle), rows)
        assertEquals(listOf("activitypub:new", "channel:reaction:me:channel:message", "activitypub:old"), feed.entries.map { it.key })
        assertEquals(notificationTimestamp("2026-10-24T10:00:00.000002+09:00"), notificationTimestamp(middle.text("created_at")))
        assertEquals(feed.entries.map { it.key }, unifiedNotificationFeed(listOf(local("local", middle.text("created_at"), true)), rows.map { kotlinx.serialization.json.JsonObject(it + ("is_read" to kotlinx.serialization.json.JsonPrimitive(true))) }).entries.map { it.key })
    }
    @Test fun duplicateCrossPostsKeepBothReadIdentitiesAndOneUnreadCount() {
        val time = "2026-10-24T01:00:00Z"
        val feed = unifiedNotificationFeed(listOf(local("local", time)), listOf(remote("mirror", time, "local:alice"), remote("external", time)))
        assertEquals(2, feed.entries.size)
        assertEquals(1, feed.duplicateUnreadCount)
        assertEquals("local", feed.mirrors.single().channelId)
        assertEquals("mirror", feed.mirrors.single().item.text("id"))
        val alreadyRead = unifiedNotificationFeed(listOf(local("local", time, true)), listOf(remote("mirror", time, "local:alice")))
        assertFalse(alreadyRead.entries.single().channel!!.single().flag("is_read"))
        assertEquals(0, alreadyRead.duplicateUnreadCount)
    }
    @Test fun holdsOlderChannelRowsBehindTheUnloadedFederatedFrontier() {
        val rows = listOf(local("old", "2026-10-23T00:00:00Z"))
        val federated = listOf(remote("remote", "2026-10-24T00:00:00Z"))
        assertEquals(listOf("activitypub:remote"), unifiedNotificationFeed(rows, federated, true).entries.map { it.key })
        assertEquals(2, unifiedNotificationFeed(rows, federated, false).entries.size)
    }
}
