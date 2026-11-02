package zoo

import androidx.compose.runtime.*
import kotlinx.coroutines.CancellationException

/** Electric owns local reactions; Home invalidations refresh the federated actor overlay. */
@Composable
fun federatedChannelReactions(app: Session, channel: String): Pair<List<Row>, Boolean> {
    var rows by remember(app, channel) { mutableStateOf<List<Row>>(emptyList()) }
    var failed by remember(app, channel) { mutableStateOf(false) }
    var revision by remember(app, channel) { mutableStateOf(0) }
    DisposableEffect(app, channel) {
        val unsubscribe = app.homeLive.subscribe { revision++ }
        onDispose { unsubscribe() }
    }
    LaunchedEffect(app, channel, revision, app.env.online, app.env.visible) {
        if (app.env.online && app.env.visible) {
            try {
                rows = app.api.request("/api/channels/$channel/federated-reactions").rows()
                failed = false
            } catch (e: CancellationException) { throw e }
            catch (_: Exception) { failed = true }
        }
    }
    return rows to failed
}
