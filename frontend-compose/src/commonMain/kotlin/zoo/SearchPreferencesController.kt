package zoo

import androidx.compose.runtime.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.jsonObject

/** Account-scoped preferences survive settings panels, and cancel with their Session. */
class SearchPreferencesController(private val app: Session, parent: CoroutineScope) {
    private val lifetime = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + lifetime)
    private var request: Job? = null
    private var generation = 0
    private var started = false
    var loading by mutableStateOf(true)
        private set

    var saving by mutableStateOf(false)
        private set

    var error by mutableStateOf("")
        private set

    fun ensureLoaded() {
        if (!started && app.config.userId.isNotEmpty()) {
            started = true
            retry()
        }
    }

    fun retry() {
        val version = ++generation
        request?.cancel()
        loading = true
        error = ""
        request =
            scope.launch {
                try {
                    val engine =
                        app.api
                            .request("/api/users/me/preferences")
                            .jsonObject
                            .text("search_engine")
                    require(engine in searchEngines)
                    if (version == generation) app.searchEngine = engine
                } catch (cause: CancellationException) {
                    throw cause
                } catch (_: Exception) {
                    if (version == generation) error = "load"
                } finally {
                    if (version == generation) loading = false
                }
            }
    }

    fun save(next: String) {
        if (
            loading ||
                saving ||
                error == "load" ||
                next == app.searchEngine ||
                next !in searchEngines ||
                !lifetime.isActive
        )
            return
        saving = true
        error = ""
        scope.launch {
            try {
                app.api.request("/api/users/me/preferences", "PATCH", obj("search_engine" to next))
                app.searchEngine = next
            } catch (cause: CancellationException) {
                throw cause
            } catch (_: Exception) {
                error = "save"
            } finally {
                saving = false
            }
        }
    }

    fun close() {
        lifetime.cancel()
    }
}
