package zoo.android

import zoo.*

/** The first synchronized snapshot is history; later unseen IDs are new arrivals. */
internal class NotificationInbox {
    private val seen = mutableSetOf<String>()
    private var initialized = false

    fun update(rows: List<Row>?): List<Row> {
        // null means no successful snapshot yet, including after an initial sync failure.
        if (rows == null) return emptyList()
        val fresh = rows.filter { seen.add(it.text("id")) && !it.flag("is_read") }
        return if (initialized) fresh else {
            initialized = true
            emptyList()
        }
    }
}
