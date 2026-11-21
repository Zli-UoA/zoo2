package zoo

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive

internal class NotificationReadQueue(private val app: Session) {
    private val pending = linkedMapOf<String, UnifiedNotificationEntry>()
    private val processing = mutableSetOf<String>()
    private var worker: Job? = null

    fun viewed(entry: UnifiedNotificationEntry) {
        if (entry.key in processing) return
        pending[entry.key] = entry
        if (worker?.isActive == true) return
        worker = app.scope.launch {
            while (pending.isNotEmpty()) {
                delay(100)
                val batch = pending.values.toList()
                processing.addAll(pending.keys)
                pending.clear()
                try {
                    write(batch)
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (_: Exception) {
                    app.toast = app.t("notifications.readError")
                } finally {
                    batch.forEach { processing.remove(it.key) }
                }
            }
        }
    }

    private suspend fun write(entries: List<UnifiedNotificationEntry>) {
        val serverIds = linkedSetOf<String>()
        val federatedIds = linkedSetOf<String>()
        for (entry in entries) {
            entry.item?.takeIf { !it.flag("is_read") }?.let { federatedIds += it.text("id") }
            for (row in entry.channel.orEmpty()) {
                if (!row.flag("is_read")) serverIds += row.text("id")
            }
            for (mirror in entry.mirrors) {
                if (!mirror.item.flag("is_read")) federatedIds += mirror.item.text("id")
            }
        }
        for (ids in serverIds.filter(String::isNotEmpty).chunked(500)) {
            app.api.request("/api/notifications/read", "POST", obj("ids" to JsonArray(ids.map(::JsonPrimitive))))
        }
        val remote = federatedIds.filter(String::isNotEmpty)
        if (remote.isNotEmpty()) app.activityPubNotifications.markRead(remote)
    }

    fun close() {
        worker?.cancel()
        pending.clear()
    }
}
