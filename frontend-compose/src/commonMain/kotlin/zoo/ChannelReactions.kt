package zoo

import androidx.compose.runtime.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay

/** Electric owns local reactions; Home invalidations refresh the federated actor overlay. */
@Composable
fun federatedChannelReactions(app: Session, channel: String): Pair<List<Row>, Boolean> {
    var rows by remember(app, channel) { mutableStateOf<List<Row>>(emptyList()) }
    var failed by remember(app, channel) { mutableStateOf(false) }
    val invalidations = remember(app, channel) { Channel<Unit>(Channel.CONFLATED) }
    DisposableEffect(app, channel) {
        val unsubscribe = app.homeLive.subscribe { invalidations.trySend(Unit) }
        onDispose { unsubscribe() }
    }
    LaunchedEffect(app, channel, app.env.online, app.env.visible) {
        if (app.env.online && app.env.visible) {
            invalidations.trySend(Unit)
            for (ignored in invalidations) {
            try {
                rows = app.api.request("/api/channels/$channel/federated-reactions").rows()
                failed = false
            } catch (e: CancellationException) { throw e }
            catch (_: Exception) { failed = true }
            delay(5000)
            }
        }
    }
    return rows to failed
}
