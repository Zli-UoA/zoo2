package zoo.android

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import androidx.core.content.ContextCompat
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import zoo.MessageCalendar

internal class AndroidMessageCalendar(private val read: () -> MessageCalendar = { MessageCalendar.current() }) {
    var value by mutableStateOf(read())
        private set

    fun refreshCalendarDay() { value = read() }

    fun observe(context: Context): () -> Unit {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                refreshCalendarDay()
            }
        }
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_DATE_CHANGED)
            addAction(Intent.ACTION_TIME_CHANGED)
            addAction(Intent.ACTION_TIMEZONE_CHANGED)
        }
        ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        refreshCalendarDay()
        return { context.unregisterReceiver(receiver) }
    }
}
