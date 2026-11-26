package zoo

import kotlin.time.Clock
import kotlin.time.Instant
import kotlinx.datetime.number
import kotlinx.datetime.LocalDate
import kotlinx.datetime.TimeZone
import kotlinx.datetime.toLocalDateTime

data class MessageCalendar(val today: LocalDate, val zone: TimeZone) {
    companion object {
        fun current(at: Instant = Clock.System.now()): MessageCalendar {
            val zone = TimeZone.currentSystemDefault()
            return MessageCalendar(at.toLocalDateTime(zone).date, zone)
        }
    }

    fun format(timestamp: String): String {
        val millis = timestampMillis(timestamp) ?: return timestamp
        val local = Instant.fromEpochMilliseconds(millis).toLocalDateTime(zone)
        val clock = "${pad(local.hour)}:${pad(local.minute)}"
        if (local.date == today) return clock
        val monthDay = "${pad(local.month.number)}/${pad(local.day)}"
        return if (local.year == today.year) "$monthDay $clock"
            else "${local.year.toString().padStart(4, '0')}/$monthDay $clock"
    }

    private fun pad(value: Int) = value.toString().padStart(2, '0')
}
