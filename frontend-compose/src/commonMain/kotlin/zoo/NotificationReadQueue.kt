package zoo

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive

internal class NotificationReadQueue(private val app: Session) {
    private val pendingChannel = linkedSetOf<String>()
    private val pendingFederated = linkedSetOf<String>()
    private var writingChannel: Set<String> = emptySet()
    private var writingFederated: Set<String> = emptySet()
    private var worker: Job? = null

    fun viewed(entry: UnifiedNotificationEntry) {
        for (row in entry.channel.orEmpty()) {
            val id = row.text("id")
            if (id.isNotEmpty() && !row.flag("is_read") && id !in writingChannel) pendingChannel += id
        }
        for (row in listOfNotNull(entry.item) + entry.mirrors.map { it.item }) {
            val id = row.text("id")
            if (id.isNotEmpty() && !row.flag("is_read") && id !in writingFederated) pendingFederated += id
        }
        if (worker?.isActive == true) return
        worker = app.scope.launch {
            while (pendingChannel.isNotEmpty() || pendingFederated.isNotEmpty()) {
                delay(100)
                writingChannel = pendingChannel.toSet()
                writingFederated = pendingFederated.toSet()
                pendingChannel.clear()
                pendingFederated.clear()
                try {
                    write(writingChannel, writingFederated)
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (_: Exception) {
                    app.toast = app.t("notifications.readError")
                } finally {
                    writingChannel = emptySet()
                    writingFederated = emptySet()
                }
            }
        }
    }

    private suspend fun write(serverIds: Set<String>, federatedIds: Set<String>) {
        for (ids in serverIds.chunked(500)) {
            app.api.request("/api/notifications/read", "POST", obj("ids" to JsonArray(ids.map(::JsonPrimitive))))
        }
        if (federatedIds.isNotEmpty()) app.activityPubNotifications.markRead(federatedIds.toList())
    }

    fun close() {
        worker?.cancel()
        pendingChannel.clear()
        pendingFederated.clear()
    }
}
