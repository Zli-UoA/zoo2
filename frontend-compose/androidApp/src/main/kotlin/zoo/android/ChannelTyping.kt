package zoo.android

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import zoo.*

/** One foreground channel or thread. Only presence is sent, including in encrypted rooms. */
internal class ChannelTyping(private val session: Session, channel: String, private val parent: String?) {
    private val path = "/api/channels/${channel.encodeURLParameter()}/typing"
    private val id = session.platform.uuid()
    private val updates = Channel<Boolean>(Channel.CONFLATED)
    private val wake = Channel<Unit>(Channel.CONFLATED)
    private var expiry: Job? = null
    private var lastStart: Long? = null
    var label by mutableStateOf("")
        private set

    init {
        session.scope.launch {
            for (active in updates) {
                try {
                    withTimeoutOrNull(5000) {
                        session.api.request(path, "POST", obj("session_id" to id, "parent_id" to parent, "active" to active))
                    }
                } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { }
            }
        }
    }

    fun edit(hasText: Boolean) {
        expiry?.cancel()
        val last = lastStart
        if (!hasText) {
            if (last != null) updates.trySend(false)
            lastStart = null
            return
        }
        val now = android.os.SystemClock.elapsedRealtime()
        if (last == null) wake.trySend(Unit)
        if (last == null || now - last >= 2000) {
            lastStart = now
            updates.trySend(true)
        }
        expiry = session.scope.launch { delay(3000); edit(false) }
    }

    suspend fun poll() {
        val query = parent?.let { "?parent_id=${it.encodeURLParameter()}" }.orEmpty()
        while (currentCoroutineContext().isActive) {
            val people = try {
                withTimeoutOrNull(5000) { session.api.request(path + query).rows().map { it.text("display_name") } }.orEmpty()
            } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { emptyList() }
            label = if (people.isEmpty()) "" else session.t("messages.typing",
                "names" to people.joinToString(session.t("messages.typingSeparator")))
            withTimeoutOrNull(if (people.isEmpty() && lastStart == null) 5000L else 1500L) { wake.receive() }
        }
    }

    fun close() {
        edit(false)
        updates.close()
    }
}
