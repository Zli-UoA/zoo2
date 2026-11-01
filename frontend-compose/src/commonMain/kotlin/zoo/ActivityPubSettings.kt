package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.*
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.*

class ActivityPubSettingsController(
    parent: CoroutineScope,
    private val request: suspend (String, Row?) -> Row,
) {
    private val lifetime = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + lifetime)
    private var read: Job? = null
    private var generation = 0
    var profile by mutableStateOf<Row?>(null)
        private set

    var name by mutableStateOf("")
        private set

    var summary by mutableStateOf("")
        private set

    var failed by mutableStateOf(false)
        private set

    var pending by mutableStateOf(false)
        private set

    var saved by mutableStateOf(false)
        private set

    val valid
        get() = unicodeLength(name.trim()) <= 100 && unicodeLength(summary) <= 1000

    fun editName(value: String) {
        if (!pending) {
            name = value
            saved = false
        }
    }

    fun editSummary(value: String) {
        if (!pending) {
            summary = value
            saved = false
        }
    }

    private fun apply(value: Row) {
        profile = value
        name = value.text("display_name")
        summary = value.text("summary")
    }

    fun load() {
        if (!lifetime.isActive) return
        val version = ++generation
        read?.cancel()
        failed = false
        read =
            scope.launch {
                try {
                    val value = request("GET", null)
                    if (version == generation) apply(value)
                } catch (cause: CancellationException) {
                    throw cause
                } catch (_: Exception) {
                    if (version == generation) failed = true
                }
            }
    }

    fun save() {
        if (!lifetime.isActive || profile == null || pending || !valid) return
        val body = obj("display_name" to name, "summary" to summary)
        pending = true
        failed = false
        saved = false
        scope.launch {
            try {
                apply(request("PUT", body))
                saved = true
            } catch (cause: CancellationException) {
                throw cause
            } catch (_: Exception) {
                failed = true
            } finally {
                pending = false
            }
        }
    }

    fun close() {
        generation++
        lifetime.cancel()
    }
}

internal fun unicodeLength(value: String): Int {
    var count = 0
    var index = 0
    while (index < value.length) {
        if (
            value[index].isHighSurrogate() &&
                index + 1 < value.length &&
                value[index + 1].isLowSurrogate()
        )
            index++
        index++
        count++
    }
    return count
}

@Composable
fun ActivityPubSettings(app: Session, state: ActivityPubSettingsController) {
    var composing by remember { mutableStateOf(false) }
    SettingsSection("ActivityPub", app.t("activitypubSettings.description")) {
        if (state.profile != null) {
            Column(
                Modifier.fillMaxWidth()
                    .then(
                        webControlModifier(
                            "form",
                            attributes = mapOf("aria-label" to "ActivityPub"),
                        )
                    ),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Field(
                    state.name,
                    app.t("activitypubSettings.displayName"),
                    Modifier.fillMaxWidth().onPreviewKeyEvent {
                        if (it.type == KeyEventType.KeyDown && it.key == Key.Enter && !composing) {
                            state.save()
                            true
                        } else false
                    },
                    placeholder = state.profile?.text("default_display_name"),
                    enabled = !state.pending,
                    inputAttributes = mapOf("aria-describedby" to "apub-display-name-hint"),
                    onComposition = { composing = it },
                ) {
                    state.editName(it)
                }
                Text(
                    app.t("activitypubSettings.nameHint"),
                    Modifier.then(
                        webControlModifier(
                            "text",
                            attributes = mapOf("id" to "apub-display-name-hint"),
                        )
                    ),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Field(
                    state.summary,
                    app.t("activitypubSettings.bio"),
                    Modifier.fillMaxWidth(),
                    singleLine = false,
                    minimumHeight = 64.dp,
                    enabled = !state.pending,
                ) {
                    state.editSummary(it)
                }
                Button(
                    app.t("home.save"),
                    primary = true,
                    enabled = !state.pending && state.valid,
                ) {
                    state.save()
                }
                if (state.saved)
                    Text(
                        app.t("activitypubSettings.saved"),
                        Modifier.then(webControlModifier("status")),
                    )
            }
        } else if (!state.failed)
            Text(app.t("activitypubSettings.loading"), Modifier.then(webControlModifier("status")))
        if (state.failed)
            CssFlowRow(Modifier.fillMaxWidth().then(webControlModifier("alert"))) {
                Text(app.t("home.controlsError"), color = MaterialTheme.colorScheme.error)
                if (state.profile == null)
                    Button(app.t("home.refresh"), variant = "link") { state.load() }
            }
    }
}
