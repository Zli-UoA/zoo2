package zoo.android

import android.content.Intent
import android.os.Looper
import java.time.LocalDateTime
import java.time.ZoneId
import kotlin.time.Instant
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import kotlin.test.assertEquals
import zoo.MessageCalendar

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class AndroidMessageCalendarTest {
    @Test
    fun dateBroadcastUpdatesAnExistingTimestamp() {
        val context = RuntimeEnvironment.getApplication()
        val zone = ZoneId.systemDefault()
        var now = LocalDateTime.of(2026, 10, 19, 19, 18).atZone(zone)
        val calendar = AndroidMessageCalendar { MessageCalendar.current(Instant.fromEpochMilliseconds(now.toInstant().toEpochMilli())) }
        val stop = calendar.observe(context)
        try {
            val post = now.toInstant().toString()
            assertEquals("19:18", calendar.value.format(post))
            now = now.plusDays(1)
            context.sendBroadcast(Intent(Intent.ACTION_DATE_CHANGED))
            shadowOf(Looper.getMainLooper()).idle()
            assertEquals("10/19 19:18", calendar.value.format(post))
        } finally { stop() }
    }
}
