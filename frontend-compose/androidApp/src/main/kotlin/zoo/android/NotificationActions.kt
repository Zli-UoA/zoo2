package zoo.android

import kotlinx.coroutines.*
import zoo.*

/** Session-owned action state; errors only update notifications which are still shown. */
internal class NotificationActions(
    private val shown: Map<String, Row>,
    private val request: suspend (String, String, Row) -> Unit,
    private val uuid: () -> String,
    private val canReply: (Row) -> Boolean,
    private val replyParent: (Row) -> String?,
    private val showError: (Row, String) -> Unit,
    private val dismiss: (String) -> Unit,
) {
    private val busy = mutableSetOf<String>()
    private val replied = mutableSetOf<String>()
    // A timeout may hide a committed POST. Retry the same body with its ID, but
    // a newly entered body is a separate message, including after another timeout.
    private val replyIds = mutableMapOf<Pair<String, String>, String>()

    fun hasReplied(id: String): Boolean = id in replied

    private fun error(id: String, key: String) {
        shown[id]?.let { showError(it, key) }
    }

    suspend fun receive(id: String, action: String, reply: String?, timeoutMillis: Long = 9_000) {
        val completed = withTimeoutOrNull(timeoutMillis) {
            perform(id, action, reply)
            true
        }
        if (completed == null) {
            currentCoroutineContext().ensureActive()
            error(id, if (action == "reply" && !hasReplied(id)) "common.writeError" else "notifications.readError")
        }
    }

    private suspend fun perform(id: String, action: String, reply: String?) {
        val row = shown[id] ?: return
        if (!busy.add(id)) return
        var errorKey = "notifications.readError"
        try {
            when (action) {
                "dismiss" -> { dismiss(id); return }
                "reply" -> {
                    // The OS may deliver an older PendingIntent even after its action was removed.
                    if (hasReplied(id)) { error(id, "notifications.readError"); return }
                    if (!canReply(row)) { error(id, "e2ee.sendUnavailable"); return }
                    val text = reply?.trim()?.takeIf { it.isNotEmpty() } ?: return
                    errorKey = "common.writeError"
                    val parent = replyParent(row) ?: run { error(id, errorKey); return }
                    request("/api/messages", "POST", obj(
                        "id" to replyIds.getOrPut(id to text, uuid),
                        "channel_id" to row.text("channel_id"),
                        "parent_id" to parent,
                        "content" to text,
                    ))
                    replied += id
                    errorKey = "notifications.readError"
                }
                "read" -> Unit
                else -> return
            }
            request("/api/notifications/$id/read", "PATCH", obj())
            dismiss(id)
        } catch (e: CancellationException) { throw e }
        catch (_: Exception) { error(id, errorKey) }
        finally { busy.remove(id) }
    }
}
