package zoo

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.input.key.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlin.time.TimeSource
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*

fun clampBubbleContent(value: String): String = buildString {
    var index = 0
    var count = 0
    while (index < value.length && count++ < 1000) {
        val ch = value[index++]
        append(ch)
        if (ch.isHighSurrogate() && index < value.length && value[index].isLowSurrogate())
            append(value[index++])
    }
}

fun shouldClearBubble(
    key: String,
    ctrl: Boolean,
    meta: Boolean,
    shift: Boolean,
    alt: Boolean,
    composing: Boolean,
) = key == "Enter" && (ctrl || meta) && !shift && !alt && !composing

/** A single serialized writer owns seq and retries the latest value, never an old draft. */
class BubbleController(private val app: Session) {
    val input = MutableStateFlow("")
    val entries = MutableStateFlow<List<Row>>(emptyList())
    val sharing = MutableStateFlow(false)
    private val session = app.platform.uuid()
    private val mutex = Mutex()
    private var seq = 0
    private var offset = 0L
    private var edited = TimeSource.Monotonic.markNow()
    private var revision = 0
    private var stoppedRevision = -1

    fun edit(value: String) {
        input.value = clampBubbleContent(value)
        edited = TimeSource.Monotonic.markNow()
        revision++
    }

    private suspend fun write(value: String): Boolean =
        mutex.withLock {
            try {
                val body = buildJsonObject {
                    put("session_id", session)
                    put("seq", ++seq)
                    put("content", value)
                    put("sent_at", (timestampMillis(app.platform.now()) ?: 0L) + offset)
                }
                val exit = buildJsonObject {
                    put("session", session)
                    put("offset", offset)
                    put(
                        "body",
                        buildJsonObject {
                            put("session_id", session)
                            put("seq", seq + 1)
                            put("content", "")
                        },
                    )
                    put(
                        "headers",
                        JsonObject(app.platform.headers().mapValues { JsonPrimitive(it.value) }),
                    )
                }
                app.platform.invoke("bubbleExit", exit.toString())
                val response =
                    withTimeout(5000) { app.api.request("/api/bubble", "POST", body).jsonObject }
                response["server_time"]?.jsonPrimitive?.longOrNull?.let {
                    offset = it - (timestampMillis(app.platform.now()) ?: 0L)
                }
                response.flag("ok") && !response.flag("stale")
            } catch (e: TimeoutCancellationException) {
                false
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                (e as? ApiException)?.body?.get("server_time")?.jsonPrimitive?.longOrNull?.let {
                    offset = it - (timestampMillis(app.platform.now()) ?: 0L)
                }
                false
            }
        }

    suspend fun run(): Unit = coroutineScope {
        try {
            launch {
                var confirmed: String? = null
                var seenRevision = revision
                var idle = false
                while (isActive) {
                    if (seenRevision != revision) {
                        seenRevision = revision
                        idle = false
                    }
                    if (edited.elapsedNow().inWholeMilliseconds >= 15000) idle = true
                    val value =
                        if (idle || revision == stoppedRevision) ""
                        else input.value.takeIf { it.isNotBlank() }.orEmpty()
                    if (value != confirmed) {
                        val sentRevision = revision
                        if (write(value)) {
                            confirmed = value
                            sharing.value = value.isNotEmpty() && sentRevision == revision
                        } else {
                            sharing.value = false
                            delay(1000)
                        }
                    }
                    delay(200)
                }
            }
            var succeeded = TimeSource.Monotonic.markNow()
            while (isActive) {
                try {
                    entries.value = withTimeout(5000) { app.api.request("/api/bubble").rows() }
                    succeeded = TimeSource.Monotonic.markNow()
                } catch (e: TimeoutCancellationException) {
                    if (succeeded.elapsedNow().inWholeMilliseconds >= 15000)
                        entries.value = emptyList()
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    if (succeeded.elapsedNow().inWholeMilliseconds >= 15000)
                        entries.value = emptyList()
                }
                delay(1000)
            }
        } finally {
            stoppedRevision = revision
            sharing.value = false
            entries.value = emptyList()
            withContext(NonCancellable) {
                withTimeoutOrNull(5000) {
                    if (write(""))
                        app.platform.invoke(
                            "bubbleExit",
                            obj("session" to session, "clear" to "true").toString(),
                        )
                }
            }
        }
    }
}

@Composable
fun BubbleChat(app: Session, users: List<Row>) {
    val controller = remember(app) { BubbleController(app) }
    val input by controller.input.collectAsState()
    val entries by controller.entries.collectAsState()
    val sharing by controller.sharing.collectAsState()
    var participants by remember { mutableStateOf(false) }
    var composing by remember { mutableStateOf(false) }
    var ownFade by remember { mutableStateOf("") }
    val displayed = remember { mutableStateMapOf<String, Row>() }
    val fading = remember { mutableStateMapOf<String, Boolean>() }
    val others =
        entries.filter {
            it.text("user_id") != app.config.userId && it.text("content").isNotBlank()
        }
    LaunchedEffect(app.env.visible, app.env.online) {
        if (app.env.visible && app.env.online) controller.run()
    }
    LaunchedEffect(others) {
        val next = others.associateBy { it.text("user_id") }
        next.forEach { (id, row) ->
            displayed[id] = row
            fading[id] = false
        }
        displayed.keys
            .toList()
            .filter { it !in next && fading[it] != true }
            .forEach { id -> fading[id] = true }
    }
    displayed.keys.toList().forEach { id ->
        LaunchedEffect(id, fading[id]) {
            if (fading[id] == true) {
                delay(3000)
                displayed.remove(id)
                fading.remove(id)
            }
        }
    }
    LaunchedEffect(ownFade) {
        if (ownFade.isNotEmpty()) {
            delay(3000)
            ownFade = ""
        }
    }
    fun clear() {
        ownFade = input.trim()
        controller.edit("")
    }
    Column(Modifier.fillMaxSize()) {
        ScreenHeader(
            "message-circle-more",
            app.t("bubbleChat.title"),
            app.t("bubbleChat.description"),
        ) {
            IconButton("users", "${app.t("bubbleChat.participants")} ${others.size + 1}") {
                participants = !participants
            }
        }
        HorizontalDivider()
        BoxWithConstraints(Modifier.weight(1f)) {
            val wide = maxWidth >= 768.dp
            Row(Modifier.fillMaxSize()) {
                Column(Modifier.weight(1f)) {
                    Column(
                        Modifier.weight(1f)
                            .fillMaxWidth()
                            .verticalScroll(rememberScrollState())
                            .padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        if (
                            displayed.isEmpty() &&
                                (!sharing || input.isBlank()) &&
                                ownFade.isEmpty()
                        ) {
                            Box(
                                Modifier.fillMaxWidth().weight(1f),
                                contentAlignment = Alignment.Center,
                            ) {
                                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                                    Text(app.t("bubbleChat.emptyTitle"))
                                    Text(
                                        app.t("bubbleChat.emptyDescription"),
                                        small = true,
                                        muted = true,
                                    )
                                }
                            }
                        }
                        displayed.forEach { (id, row) ->
                            BubbleRow(
                                app,
                                users.find { it.text("id") == id } ?: row,
                                row.text("content"),
                                false,
                                fading[id] == true,
                            )
                        }
                        if ((sharing && input.isNotBlank()) || ownFade.isNotEmpty())
                            BubbleRow(
                                app,
                                users.find { it.text("id") == app.config.userId },
                                ownFade.ifEmpty { input },
                                true,
                                ownFade.isNotEmpty(),
                            )
                    }
                    Column(
                        Modifier.fillMaxWidth()
                            .border(1.dp, MaterialTheme.colorScheme.outline)
                            .padding(12.dp)
                    ) {
                        Field(
                            input,
                            app.t("bubbleChat.inputLabel"),
                            Modifier.fillMaxWidth().onPreviewKeyEvent {
                                if (
                                    it.type == KeyEventType.KeyDown &&
                                        shouldClearBubble(
                                            if (it.key == Key.Enter) "Enter" else "",
                                            it.isCtrlPressed,
                                            it.isMetaPressed,
                                            it.isShiftPressed,
                                            it.isAltPressed,
                                            composing,
                                        )
                                ) {
                                    clear()
                                    true
                                } else false
                            },
                            singleLine = false,
                            readOnly = !app.env.online,
                            onComposition = { composing = it },
                        ) {
                            ownFade = ""
                            controller.edit(it)
                        }
                        Row(
                            Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                "${input.codePointCount()}/1000",
                                Modifier.weight(1f),
                                small = true,
                                muted = true,
                            )
                            Button(app.t("bubbleChat.clear"), input.isNotEmpty()) { clear() }
                        }
                        if (!app.env.touch)
                            Text(app.t("bubbleChat.hint"), small = true, muted = true)
                    }
                }
                if (participants && wide)
                    Column(
                        Modifier.width(256.dp)
                            .fillMaxHeight()
                            .verticalScroll(rememberScrollState())
                            .padding(16.dp)
                    ) {
                        BubbleParticipants(app, users, others)
                    }
            }
            if (participants && !wide)
                Panel(app.t("bubbleChat.participants"), { participants = false }) {
                    BubbleParticipants(app, users, others)
                }
        }
    }
}

@Composable
private fun BubbleParticipants(app: Session, users: List<Row>, others: List<Row>) {
    Text(app.t("bubbleChat.participants"), weight = FontWeight.SemiBold)
    (listOf(users.find { it.text("id") == app.config.userId } ?: obj("id" to app.config.userId)) +
            others)
        .forEach { row ->
            Row(
                Modifier.padding(vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Avatar(row)
                Text(author(row).ifEmpty { row.text("id") }, small = true)
            }
        }
}

@Composable
private fun BubbleRow(app: Session, user: Row?, content: String, mine: Boolean, fading: Boolean) {
    val opacity by
        animateFloatAsState(if (fading) 0f else 1f, tween(if (app.env.reducedMotion) 0 else 3000))
    Row(
        Modifier.fillMaxWidth().alpha(opacity),
        horizontalArrangement = if (mine) Arrangement.End else Arrangement.Start,
    ) {
        if (!mine) Avatar(user)
        Column(Modifier.widthIn(max = 560.dp).padding(horizontal = 12.dp)) {
            Text(user?.let(::author).orEmpty(), small = true, muted = true)
            Surface(
                shape = RoundedCornerShape(16.dp),
                color =
                    if (mine) MaterialTheme.colorScheme.secondaryContainer
                    else MaterialTheme.colorScheme.surfaceVariant,
            ) {
                Text(content, Modifier.padding(12.dp))
            }
        }
    }
}
