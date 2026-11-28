package zoo.android

import kotlinx.serialization.json.JsonObject
import zoo.Row
import zoo.obj

/** Each entry owns its state; Back must not retain metadata from the page being left. */
internal class NavigationHistory(initialPath: String = "/home") {
    data class Entry(val path: String, val state: Row)

    private val entries = mutableListOf(entry(initialPath, JsonObject(emptyMap()), 0))
    val current: Entry get() = entries.last()
    var revision = 0L
        private set

    private fun entry(path: String, state: Row, index: Int) =
        Entry(path, JsonObject(state + obj("__TSR_index" to index)))

    fun navigate(path: String, state: Row = JsonObject(emptyMap())): Entry {
        if (path == current.path) return current
        revision++
        return entry(path, state, entries.size).also { entries.add(it) }
    }

    fun replace(path: String): Entry {
        if (path != current.path) revision++
        return entry(path, current.state, entries.lastIndex).also { entries[entries.lastIndex] = it }
    }

    fun back(): Entry? {
        if (entries.size == 1) return null
        entries.removeAt(entries.lastIndex)
        revision++
        return current
    }
}
