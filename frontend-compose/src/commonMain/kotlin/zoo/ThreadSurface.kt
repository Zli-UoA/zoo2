package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.jsonObject

@Composable
internal fun ThreadSurface(
    app: Session,
    count: Int,
    loading: Boolean,
    modifier: Modifier = Modifier,
    close: () -> Unit,
    root: @Composable () -> Unit,
    replies: @Composable ColumnScope.() -> Unit,
    composer: (@Composable () -> Unit)?,
) {
    val windowWidth =
        with(LocalDensity.current) { LocalWindowInfo.current.containerSize.width.toDp() }
    val compact = windowWidth < 640.dp
    Column(
        modifier
            .then(
                webControlModifier(
                    "complementary",
                    attributes = mapOf("aria-label" to app.t("threads.title")),
                )
            )
            .then(
                if (compact) Modifier.fillMaxWidth()
                else Modifier.width(if (windowWidth >= 1280.dp) 384.dp else 320.dp)
            )
            .fillMaxHeight()
            .background(MaterialTheme.colorScheme.background)
    ) {
        PaneHeader {
            if (compact)
                IconButton("arrow-left", app.t("threads.close"), size = 44.dp, click = close)
            Column(Modifier.weight(1f)) {
                Text(app.t("threads.title"), weight = FontWeight.SemiBold)
                Text(
                    app.t(
                        if (loading) "threads.loading" else "threads.replyCount",
                        "count" to count.toString(),
                    ),
                    small = true,
                    muted = true,
                )
            }
            if (!compact) IconButton("x", app.t("threads.close"), size = 28.dp, click = close)
        }
        ThreadActions(app, canParticipate = composer != null, replyCount = count)
        HorizontalDivider()
        Box(
            Modifier.fillMaxWidth()
                .heightIn(max = 240.dp)
                .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .3f))
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 8.dp, vertical = 8.dp)
        ) {
            root()
        }
        HorizontalDivider()
        Column(Modifier.weight(1f), content = replies)
        composer?.invoke()
    }
}


/** Only the open pane fetches personal membership; no per-message requests. */
@Composable
private fun ThreadActions(app: Session, canParticipate: Boolean, replyCount: Int) {
    val id = app.thread ?: return
    var detail by remember(app, id) { mutableStateOf<Row?>(null) }
    var failed by remember(app, id) { mutableStateOf(false) }
    val mutex = remember(app, id) { Mutex() }
    val pending = "thread:$id" in app.pending
    LaunchedEffect(app, id, app.env.visible, app.env.online, replyCount) {
        if (!app.env.visible || !app.env.online) return@LaunchedEffect
        while (isActive) {
            try {
                mutex.withLock {
                    // 参加者の人数と一覧は表示しないので、O(P) の読み取りを省く。
                    val result = app.api.request("/api/threads/$id?participants=none").jsonObject["data"]!!.jsonObject
                    detail = result
                    failed = false
                    if (app.thread == id && result.text("id") != id) app.thread = result.text("id")
                }
            } catch (e: CancellationException) { throw e }
            catch (_: Exception) { failed = true }
            delay(60_000)
        }
    }
    fun change(action: String) {
        app.run("thread:$id") {
            mutex.withLock {
                detail = app.api.request("/api/threads/$id/$action?participants=none", "POST", obj()).jsonObject["data"]!!.jsonObject
                failed = false
            }
        }
    }
    if (canParticipate) Column(Modifier.padding(horizontal = 12.dp, vertical = 6.dp)) {
        val archived = detail?.present("thread_archive_at") == true
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (archived) Button(app.t("threads.reopen"), !pending && app.env.online, size = "sm") { change("unarchive") }
            if (detail?.text("membership") == "joined")
                Button(app.t("threads.leave"), !pending && app.env.online, variant = "ghost", size = "sm") { change("leave") }
            else if (!archived)
                Button(app.t("threads.join"), detail != null && !pending && app.env.online, size = "sm") { change("join") }
        }
        // 参加行があるスレッドは、参加してから返信する（未参加のまま送ると 403 になる）。
        val membership = detail?.text("membership")
        val needsJoin = membership != null && membership != "joined" && membership.isNotEmpty() &&
            (membership == "left" || detail?.flag("tracked") == true)
        if (needsJoin) Text(app.t("threads.joinToReply"), small = true, muted = true)
        if (archived) Text(app.t("threads.reopenOnSend"), small = true, muted = true)
        if (failed) Text(app.t("threads.membershipError"), small = true, color = MaterialTheme.colorScheme.error)
    }
}
