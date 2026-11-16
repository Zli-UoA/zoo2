package zoo.android

import android.icu.text.RelativeDateTimeFormatter
import android.icu.text.RelativeDateTimeFormatter.RelativeUnit
import java.util.Locale
import kotlin.math.abs
import zoo.timestampMillis

/** ICU supplies all supported UI languages, including future dates and plural forms. */
internal fun androidRelativeTimestamp(timestamp: String, now: Long, locale: String): String? {
    val millis = timestampMillis(timestamp) ?: return null
    val seconds = (millis.toDouble() - now.toDouble()) / 1000
    val (divisor, unit) = when {
        abs(seconds) < 60 -> 1 to RelativeUnit.SECONDS
        abs(seconds) < 3600 -> 60 to RelativeUnit.MINUTES
        abs(seconds) < 86400 -> 3600 to RelativeUnit.HOURS
        abs(seconds) < 604800 -> 86400 to RelativeUnit.DAYS
        abs(seconds) < 2592000 -> 604800 to RelativeUnit.WEEKS
        abs(seconds) < 31536000 -> 2592000 to RelativeUnit.MONTHS
        else -> 31536000 to RelativeUnit.YEARS
    }
    return RelativeDateTimeFormatter.getInstance(Locale.forLanguageTag(locale))
        .format(abs(seconds / divisor).toLong().toDouble(),
            if (seconds > 0) RelativeDateTimeFormatter.Direction.NEXT else RelativeDateTimeFormatter.Direction.LAST,
            unit)
}
