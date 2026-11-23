package zoo

import androidx.compose.runtime.*
import kotlin.time.TimeSource
import kotlinx.coroutines.*

enum class ToastKind {
    Default,
    Success,
    Error,
}

data class NativeToastState(
    val serial: Long,
    val message: String,
    val kind: ToastKind,
    /** null is Sonner's Infinity duration. */
    val durationMillis: Long?,
)

private val toastEpoch = TimeSource.Monotonic.markNow()

/** The product and catalog use the same serial-safe lifetime and pause state. */
class NativeToastController(
    private val scope: CoroutineScope,
    private val clockMillis: () -> Long = { toastEpoch.elapsedNow().inWholeMilliseconds },
) {
    var current by mutableStateOf<NativeToastState?>(null)
        private set

    private var serial = 0L
    private var remainingMillis: Long? = null
    private var startedAt: Long? = null
    private var timer: Job? = null
    private var hovered = false
    private var visible = true
    private var closed = false

    fun show(message: String, kind: ToastKind = ToastKind.Default, durationMillis: Long? = 4_000L) {
        if (closed) return
        stopTimer()
        current = NativeToastState(++serial, message, kind, durationMillis)
        remainingMillis = durationMillis?.coerceAtLeast(0L)
        hovered = false
        startTimer()
    }

    fun dismiss(expectedSerial: Long? = null) {
        if (expectedSerial != null && current?.serial != expectedSerial) return
        stopTimer()
        current = null
        remainingMillis = null
        hovered = false
    }

    fun setHovered(expectedSerial: Long, value: Boolean) {
        if (current?.serial != expectedSerial || hovered == value) return
        stopTimer()
        hovered = value
        startTimer()
    }

    fun setVisible(value: Boolean) {
        if (visible == value) return
        stopTimer()
        visible = value
        startTimer()
    }

    private fun stopTimer() {
        timer?.cancel()
        timer = null
        startedAt?.let { start ->
            remainingMillis =
                remainingMillis?.let {
                    (it - (clockMillis() - start).coerceAtLeast(0L)).coerceAtLeast(0L)
                }
        }
        startedAt = null
    }

    private fun startTimer() {
        val entry = current ?: return
        val remaining = remainingMillis ?: return
        if (hovered || !visible) return
        startedAt = clockMillis()
        timer =
            scope.launch {
                delay(remaining)
                if (current?.serial == entry.serial) dismiss(entry.serial)
            }
    }

    fun close() {
        dismiss()
        closed = true
    }
}
