package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.*

/** The settings panel owns its requests, including an in-flight read or save on disposal. */
class ChannelTimelineSettingsController(
    parent: CoroutineScope,
    private val request: suspend (method: String, body: Row?) -> Row,
) {
    private val lifetime = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + lifetime)
    private var read: Job? = null
    private var generation = 0
    var canManage by mutableStateOf(false)
        private set

    var pattern by mutableStateOf("")
        private set

    var failed by mutableStateOf(false)
        private set

    var pending by mutableStateOf(false)
        private set

    fun edit(value: String) {
        if (canManage && !pending) pattern = value
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
                    if (version == generation) {
                        canManage = value.flag("can_manage")
                        pattern = value.text("ltl_channel_pattern")
                    }
                } catch (cause: CancellationException) {
                    throw cause
                } catch (_: Exception) {
                    if (version == generation) failed = true
                }
            }
    }

    fun save() {
        if (!lifetime.isActive || !canManage || pending || pattern.length > 200) return
        val next = pattern
        pending = true
        failed = false
        scope.launch {
            try {
                request("PUT", obj("ltl_channel_pattern" to next))
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

@Composable
fun ChannelTimelineSettings(app: Session, state: ChannelTimelineSettingsController) {
    if (!state.failed && !state.canManage) return
    var composing by remember { mutableStateOf(false) }
    SettingsSection(app.t("home.channelRule"), app.t("home.ruleHint")) {
        if (state.canManage)
            CssFlowRow(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                verticalGap = 8.dp,
            ) {
                Field(
                    state.pattern,
                    app.t("home.channelRule"),
                    modifier =
                        // Match the browser input's default 20-column intrinsic width.
                        Modifier.width(210.dp).onPreviewKeyEvent {
                            if (
                                it.type == KeyEventType.KeyDown &&
                                    it.key == Key.Enter &&
                                    !it.isCtrlPressed &&
                                    !it.isMetaPressed &&
                                    !it.isAltPressed &&
                                    !it.isShiftPressed &&
                                    !composing
                            ) {
                                state.save()
                                true
                            } else false
                        },
                    showLabel = false,
                    placeholder = "public-%",
                    enabled = !state.pending,
                    maxLength = 200,
                    onComposition = { composing = it },
                ) {
                    state.edit(it)
                }
                Button(app.t("home.save"), enabled = !state.pending, variant = "outline") {
                    if (!composing) state.save()
                }
            }
        if (state.failed)
            CssFlowRow(
                Modifier.fillMaxWidth().then(webControlModifier("alert")).semantics {
                    liveRegion = LiveRegionMode.Polite
                },
                verticalAlignment = Alignment.CenterVertically,
                horizontalGap = 3.dp,
            ) {
                Text(
                    app.t("home.controlsError"),
                    small = true,
                    color = MaterialTheme.colorScheme.error,
                )
                Button(app.t("home.refresh"), variant = "link") { state.load() }
            }
    }
}
