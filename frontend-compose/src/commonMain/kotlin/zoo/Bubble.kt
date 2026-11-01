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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
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
fun BubbleChat(app: Session, users: List<Row>, initialParticipants: Boolean = false) {
    val controller = remember(app) { BubbleController(app) }
    val input by controller.input.collectAsState()
    val entries by controller.entries.collectAsState()
    val sharing by controller.sharing.collectAsState()
    var participants by remember { mutableStateOf(initialParticipants) }
    var composing by remember { mutableStateOf(false) }
    var ownFade by remember { mutableStateOf("") }
    val displayed = remember { mutableStateMapOf<String, Row>() }
    val displayedOrder = remember { mutableStateListOf<String>() }
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
        val priorOrder = displayedOrder.toList()
        displayedOrder.clear()
        displayedOrder.addAll(next.keys)
        displayedOrder.addAll(priorOrder.filter { it !in next })
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
                displayedOrder.remove(id)
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
            Spacer(Modifier.width(12.dp))
            Button(
                (others.size + 1).toString(),
                variant = if (participants) "secondary" else "outline",
                size = "sm",
                icon = "users",
                iconSize = 16.dp,
                contentGap = 6.dp,
                textStyle =
                    TextStyle(
                        fontSize = 12.sp,
                        lineHeight = 16.sp,
                        fontWeight = FontWeight.SemiBold,
                    ),
                modifier =
                    Modifier.semantics {
                        contentDescription =
                            "${app.t("bubbleChat.participants")} ${others.size + 1}"
                    },
            ) {
                participants = !participants
            }
        }
        HorizontalDivider()
        BoxWithConstraints(Modifier.weight(1f)) {
            val wide = maxWidth >= 768.dp
            Row(Modifier.fillMaxSize()) {
                Column(Modifier.weight(1f)) {
                    Box(Modifier.weight(1f).fillMaxWidth().padding(16.dp)) {
                        if (displayed.isEmpty() && input.isBlank() && ownFade.isEmpty())
                            Column(
                                Modifier.align(Alignment.Center).widthIn(max = 448.dp),
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Box(
                                    Modifier.size(48.dp)
                                        .background(
                                            MaterialTheme.colorScheme.surfaceVariant,
                                            RoundedCornerShape(16.dp),
                                        ),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Icon("message-circle-more", modifier = Modifier.size(24.dp))
                                }
                                Text(
                                    app.t("bubbleChat.emptyTitle"),
                                    weight = androidx.compose.ui.text.font.FontWeight.Medium,
                                )
                                Text(
                                    app.t("bubbleChat.emptyDescription"),
                                    small = true,
                                    muted = true,
                                )
                            }
                        else
                            Column(
                                Modifier.align(Alignment.TopCenter)
                                    .then(webControlModifier("list"))
                                    .widthIn(max = 768.dp)
                                    .fillMaxWidth()
                                    .verticalScroll(rememberScrollState()),
                                verticalArrangement = Arrangement.spacedBy(16.dp),
                            ) {
                                displayedOrder.forEach { id ->
                                    val row = displayed[id] ?: return@forEach
                                    BubbleRow(
                                        app,
                                        JsonObject(row + ("id" to JsonPrimitive(id))),
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
                    }
                    HorizontalDivider()
                    Column(Modifier.fillMaxWidth().padding(12.dp)) {
                        Column(
                            Modifier.widthIn(max = 768.dp)
                                .fillMaxWidth()
                                .align(Alignment.CenterHorizontally)
                        ) {
                            ComposerFrame {
                                Field(
                                    input,
                                    app.t("bubbleChat.inputLabel"),
                                    Modifier.weight(1f).onPreviewKeyEvent {
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
                                    chrome = false,
                                    placeholder = app.t("bubbleChat.placeholder"),
                                    minimumHeight = 36.dp,
                                    contentPadding =
                                        PaddingValues(horizontal = 8.dp, vertical = 6.dp),
                                    readOnly = !app.env.online,
                                    onComposition = { composing = it },
                                ) {
                                    ownFade = ""
                                    controller.edit(it)
                                }
                                Button(
                                    app.t("bubbleChat.clear"),
                                    input.isNotBlank(),
                                    variant = "default",
                                    size = "icon",
                                    icon = "eraser",
                                    modifier =
                                        Modifier.padding(bottom = 4.dp)
                                            .size(if (wide) 36.dp else 44.dp),
                                ) {
                                    clear()
                                }
                            }
                            if (!app.env.touch)
                                Text(
                                    app.t("bubbleChat.hint"),
                                    Modifier.padding(start = 4.dp, top = 6.dp),
                                    small = true,
                                    muted = true,
                                    size = 11.sp,
                                    lineHeight = 16.5.sp,
                                )
                        }
                    }
                }

                if (participants && wide)
                    Column(
                        Modifier.width(288.dp)
                            .fillMaxHeight()
                            .then(webControlModifier("complementary"))
                            .semantics { contentDescription = app.t("bubbleChat.participants") }
                            .border(1.dp, MaterialTheme.colorScheme.outline)
                    ) {
                        PaneHeader {
                            Text(
                                app.t(
                                    "bubbleChat.participantsTitle",
                                    "count" to (others.size + 1).toString(),
                                ),
                                Modifier.weight(1f),
                                weight = FontWeight.SemiBold,
                            )
                            IconButton("x", app.t("bubbleChat.closeParticipants"), size = 24.dp) {
                                participants = false
                            }
                        }
                        HorizontalDivider()
                        BubbleParticipants(
                            app,
                            users,
                            others,
                            (sharing && input.isNotBlank()) || ownFade.isNotEmpty(),
                        )
                    }
            }
            if (participants && !wide)
                Panel(
                    app.t("bubbleChat.participantsTitle", "count" to (others.size + 1).toString()),
                    { participants = false },
                    320.dp,
                ) {
                    BubbleParticipants(
                        app,
                        users,
                        others,
                        (sharing && input.isNotBlank()) || ownFade.isNotEmpty(),
                    )
                }
        }
    }
}

@Composable
private fun BubbleParticipants(
    app: Session,
    users: List<Row>,
    others: List<Row>,
    selfTyping: Boolean,
) {
    val me =
        users.find { it.text("id") == app.config.userId }
            ?: obj("id" to app.config.userId, "display_name" to app.t("bubbleChat.you"))
    Column(
        Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(8.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        (listOf(me) +
                others.map { row ->
                    JsonObject(row + ("id" to JsonPrimitive(row.text("user_id"))))
                })
            .forEachIndexed { index, person ->
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 8.dp),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Avatar(person)
                    Column(Modifier.weight(1f)) {
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                author(person),
                                small = true,
                                weight = FontWeight.SemiBold,
                                maxLines = 1,
                            )
                            if (index == 0)
                                Text(
                                    app.t("bubbleChat.you"),
                                    Modifier.background(
                                            MaterialTheme.colorScheme.surfaceVariant,
                                            RoundedCornerShape(4.dp),
                                        )
                                        .padding(horizontal = 6.dp, vertical = 1.dp),
                                    small = true,
                                    muted = true,
                                    size = 10.sp,
                                    lineHeight = 15.sp,
                                    weight = FontWeight.Medium,
                                )
                        }
                        if (person.text("user_name").isNotEmpty())
                            Text(
                                "@${person.text("user_name")}",
                                small = true,
                                muted = true,
                                size = 11.sp,
                                lineHeight = 16.5.sp,
                            )
                    }
                    val typing = index != 0 || selfTyping
                    Row(
                        Modifier.background(
                                if (typing) MaterialTheme.colorScheme.secondaryContainer
                                else MaterialTheme.colorScheme.surfaceVariant,
                                RoundedCornerShape(50),
                            )
                            .padding(horizontal = 8.dp, vertical = 2.dp),
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        if (typing)
                            Box(
                                Modifier.size(6.dp)
                                    .background(
                                        MaterialTheme.colorScheme.secondary,
                                        androidx.compose.foundation.shape.CircleShape,
                                    )
                            )
                        Text(
                            app.t(if (typing) "bubbleChat.typing" else "bubbleChat.active"),
                            small = true,
                            muted = !typing,
                            size = 10.sp,
                            lineHeight = 15.sp,
                            weight = if (typing) FontWeight.Medium else FontWeight.Normal,
                        )
                    }
                }
            }
    }
}

@Composable
private fun BubbleRow(app: Session, user: Row?, content: String, mine: Boolean, fading: Boolean) {
    val opacity by
        animateFloatAsState(if (fading) 0f else 1f, tween(if (app.env.reducedMotion) 0 else 3000))
    BoxWithConstraints(Modifier.fillMaxWidth().then(webControlModifier("listitem"))) {
        val bubbleMaxWidth = maxWidth * .75f
        Row(
            Modifier.fillMaxWidth().alpha(opacity),
            horizontalArrangement = if (mine) Arrangement.End else Arrangement.Start,
            verticalAlignment = Alignment.Top,
        ) {
            if (!mine) Avatar(user, Modifier.padding(top = 4.dp))
            Spacer(Modifier.width(if (mine) 0.dp else 12.dp))
            Column(
                Modifier.widthIn(max = bubbleMaxWidth),
                horizontalAlignment = if (mine) Alignment.End else Alignment.Start,
            ) {
                Text(
                    user?.let(::author).orEmpty(),
                    Modifier.padding(start = 4.dp, end = 4.dp, bottom = 4.dp),
                    small = true,
                    muted = true,
                    weight = androidx.compose.ui.text.font.FontWeight.Medium,
                )
                Surface(
                    shape =
                        RoundedCornerShape(
                            topStart = if (mine) 16.dp else 4.dp,
                            topEnd = if (mine) 4.dp else 16.dp,
                            bottomStart = 16.dp,
                            bottomEnd = 16.dp,
                        ),
                    color =
                        if (mine) MaterialTheme.colorScheme.secondary
                        else MaterialTheme.colorScheme.surface,
                    border =
                        if (mine) null else BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                    shadowElevation = 1.dp,
                ) {
                    Text(
                        content,
                        Modifier.padding(
                            horizontal = if (mine) 14.dp else 15.dp,
                            vertical = if (mine) 10.dp else 11.dp,
                        ),
                        color =
                            if (mine) if (app.env.dark) Color.Black else Color.White
                            else MaterialTheme.colorScheme.onSurface,
                        lineHeight = 22.75.sp,
                    )
                }
            }
            if (mine) {
                Spacer(Modifier.width(12.dp))
                Avatar(user, Modifier.padding(top = 4.dp))
            }
        }
    }
}
