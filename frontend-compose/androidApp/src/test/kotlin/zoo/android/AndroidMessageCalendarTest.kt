package zoo.android

import android.app.Activity
import android.content.Intent
import android.os.Looper
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import java.time.LocalDateTime
import java.time.ZoneId
import kotlin.time.Instant
import java.util.TimeZone
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
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

    @Test
    fun timeZoneBroadcastUpdatesAnExistingTimestampOnTheSameDate() {
        val context = RuntimeEnvironment.getApplication()
        val originalZone = TimeZone.getDefault()
        val now = Instant.parse("2026-11-26T12:00:00Z")
        val post = "2026-07-01T12:00:00Z"
        try {
            TimeZone.setDefault(TimeZone.getTimeZone("America/Denver"))
            val calendar = AndroidMessageCalendar { MessageCalendar.current(now) }
            val stop = calendar.observe(context)
            try {
                assertEquals("07/01 06:00", calendar.value.format(post))
                TimeZone.setDefault(TimeZone.getTimeZone("America/Phoenix"))
                context.sendBroadcast(Intent(Intent.ACTION_TIMEZONE_CHANGED))
                shadowOf(Looper.getMainLooper()).idle()
                assertEquals("07/01 05:00", calendar.value.format(post))
            } finally {
                stop()
            }
        } finally {
            TimeZone.setDefault(originalZone)
        }
    }

    @Test
    fun becomingVisibleRefreshesTheTimeZoneWithoutABroadcast() {
        val originalZone = TimeZone.getDefault()
        val controller = Robolectric.buildActivity(Activity::class.java).setup()
        val activity = controller.get()
        val http = HttpClient(MockEngine { error("Unexpected HTTP request") })
        try {
            TimeZone.setDefault(TimeZone.getTimeZone("America/Denver"))
            val platform = AndroidPlatform(
                activity, http, DeviceAuth(http), "https://zoo.example", "user",
                logout = {}, chooseFiles = { emptyList() }, recentPhotos = RecentPhotos(activity),
                requestRecentPhotos = {}, confirmUnbackedKeys = { true },
            )
            try {
                val post = "2001-07-01T12:00:00Z"
                assertEquals("2001/07/01 06:00", platform.messageTimestamp(post))
                platform.visible(false)
                TimeZone.setDefault(TimeZone.getTimeZone("America/Phoenix"))
                assertEquals("2001/07/01 06:00", platform.messageTimestamp(post))
                platform.visible(true)
                assertEquals("2001/07/01 05:00", platform.messageTimestamp(post))
            } finally {
                platform.close()
            }
        } finally {
            TimeZone.setDefault(originalZone)
            http.close()
            controller.pause().stop().destroy()
        }
    }
}
