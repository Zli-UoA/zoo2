package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.input.key.*
import androidx.compose.ui.unit.*
import io.ktor.http.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

private fun noteData(value: JsonElement): Row =
    ((value as? JsonObject)?.get("data") as? JsonObject) ?: value.objectOrEmpty()

private fun noteBody(title: String, content: String, version: Int? = null) = buildJsonObject {
    put("title", title)
    put("content", content)
    if (version != null) put("version", version)
}

internal const val CHANNEL_NOTE_TITLE_MAX_LENGTH = 200
internal const val CHANNEL_NOTE_CONTENT_MAX_LENGTH = 100_000

internal enum class ChannelNoteSaveAction { Reject, QueueTaskDraft, Persist }

internal fun channelNoteSaveAction(canWrite: Boolean, busy: Boolean, hasTaskContent: Boolean): ChannelNoteSaveAction = when {
    !canWrite -> ChannelNoteSaveAction.Reject
    busy && hasTaskContent -> ChannelNoteSaveAction.QueueTaskDraft
    busy -> ChannelNoteSaveAction.Reject
    else -> ChannelNoteSaveAction.Persist
}

internal class ChannelNoteRouteGeneration(initialChannelId: String, initialNoteId: String?) {
    private var currentChannelId = initialChannelId
    private var currentNoteId = initialNoteId
    var generation: Int = 0
        private set

    fun observe(channelId: String, noteId: String?) {
        if (channelId != currentChannelId || noteId != currentNoteId) {
            currentChannelId = channelId
            currentNoteId = noteId
            generation++
        }
    }

    fun matches(channelId: String, noteId: String?, generation: Int): Boolean =
        channelId == currentChannelId && noteId == currentNoteId && generation == this.generation
}

internal fun canApplyChannelNoteResponse(
    route: ChannelNoteRouteGeneration,
    channelId: String,
    requestNoteId: String,
    requestGeneration: Int,
    selectedNoteId: String?,
): Boolean = route.matches(channelId, requestNoteId, requestGeneration) && selectedNoteId == requestNoteId

internal data class ChannelNoteTabSwitchState(
    val sourceNoteId: String? = null,
    val sourceRouteGeneration: Int = 0,
    val targetTab: String? = null,
    val pending: Boolean = false,
) {
    fun matches(noteId: String?, routeGeneration: Int): Boolean =
        pending && sourceNoteId == noteId && sourceRouteGeneration == routeGeneration

    fun forCurrentRoute(noteId: String?, routeGeneration: Int): ChannelNoteTabSwitchState =
        if (matches(noteId, routeGeneration)) this else Empty

    companion object {
        val Empty = ChannelNoteTabSwitchState()
        fun pending(noteId: String?, routeGeneration: Int, targetTab: String?) =
            ChannelNoteTabSwitchState(noteId, routeGeneration, targetTab, pending = true)
    }
}

private fun notePath(app: Session, noteId: String?): String {
    val url = URLBuilder("https://zoo.invalid${app.env.path}")
    url.parameters.remove("note")
    if (noteId != null) url.parameters.append("note", noteId)
    val query = url.build().encodedQuery
    return url.encodedPath + if (query.isEmpty()) "" else "?$query"
}

internal data class ChannelNoteLatestFetchState(
    val noteId: String? = null,
    val failed: Boolean = false,
    val error: String = "",
    val forDelete: Boolean = false,
) {
    fun canRetry(routeNoteId: String?, selectedNoteId: String?): Boolean =
        failed && noteId != null && noteId == routeNoteId && noteId == selectedNoteId

    fun forCurrentNote(currentNoteId: String?): ChannelNoteLatestFetchState =
        if (noteId == currentNoteId) this else Empty

    fun clear() = Empty

    fun markFailed(noteId: String, error: String, forDelete: Boolean) =
        ChannelNoteLatestFetchState(noteId, failed = true, error = error, forDelete = forDelete)

    fun markLoaded(noteId: String, forDelete: Boolean) =
        ChannelNoteLatestFetchState(noteId, forDelete = forDelete)

    companion object {
        val Empty = ChannelNoteLatestFetchState()
    }
}

internal data class ChannelNoteDeleteConfirmation(val targetNoteId: String? = null) {
    fun confirmableNoteId(routeNoteId: String?, selectedNoteId: String?): String? =
        targetNoteId?.takeIf { it == routeNoteId && it == selectedNoteId }

    fun forCurrentNote(currentNoteId: String?): ChannelNoteDeleteConfirmation =
        if (targetNoteId == currentNoteId) this else Empty

    companion object {
        val Empty = ChannelNoteDeleteConfirmation()
        fun forNote(noteId: String) = ChannelNoteDeleteConfirmation(noteId)
    }
}

internal data class ChannelNoteDraftState(
    val title: String,
    val content: String,
    val editing: Boolean,
    val taskDraft: String?,
) {
    fun onWriteAccessLost(savedContent: String): ChannelNoteDraftState {
        if (taskDraft == null) return this
        return copy(content = if (editing) content else savedContent)
    }

    fun useLatest(title: String, content: String) =
        copy(title = title, content = content, editing = false, taskDraft = null)
}

internal fun canPersistChannelNote(canWrite: Boolean, busy: Boolean): Boolean = canWrite && !busy

/** Summary refresh is independent of the active tab and never interrupts another load or write. */
internal fun shouldPollChannelNoteSummary(loadInFlight: Boolean, busy: Boolean): Boolean =
    !loadInFlight && !busy

internal fun shouldRefreshChannelNoteDetail(
    summaryVersion: Int?,
    selectedVersion: Int?,
    editing: Boolean,
    hasTaskDraft: Boolean,
    busy: Boolean,
): Boolean = summaryVersion != null && summaryVersion != selectedVersion && !editing && !hasTaskDraft && !busy

internal fun shouldReportMissingChannelNote(
    notePresent: Boolean,
    editing: Boolean,
    hasTaskDraft: Boolean,
    busy: Boolean,
): Boolean = !notePresent && !editing && !hasTaskDraft && !busy

internal fun canApplyChannelNoteSummaryDetail(
    route: ChannelNoteRouteGeneration,
    channelId: String,
    requestNoteId: String,
    requestGeneration: Int,
    selectedVersionAtRequest: Int?,
    currentSelectedVersion: Int?,
    selectedNoteId: String?,
    editing: Boolean,
    hasTaskDraft: Boolean,
    busy: Boolean,
): Boolean = route.matches(channelId, requestNoteId, requestGeneration) &&
    (selectedNoteId == requestNoteId || (selectedNoteId == null && selectedVersionAtRequest == null)) &&
    currentSelectedVersion == selectedVersionAtRequest && !editing && !hasTaskDraft && !busy

/** Preserve the message composition while another tab is shown. */
@Composable
fun ChannelNotes(
    app: Session,
    channel: Row,
    joined: Boolean,
    wide: Boolean,
    users: List<Row>,
    members: List<Row>,
    messages: @Composable () -> Unit,
) {
    val channelId = channel.text("id")
    val path = "/api/channels/$channelId/notes"
    val noteId = app.query["note"]
    val routeGeneration = remember(channelId) { ChannelNoteRouteGeneration(channelId, noteId) }
    routeGeneration.observe(channelId, noteId)
    val canWrite = joined && !channel.present("archive_at") &&
        (channel.text("posting_policy") != "managers" || canManage(channel, app.config.userId, app.config.isAdmin || app.config.userId in app.admins))
    var notes by remember(channelId) { mutableStateOf<List<Row>>(emptyList()) }
    var selected by remember(channelId) { mutableStateOf<Row?>(null) }
    var deleted by remember(channelId) { mutableStateOf<List<Row>>(emptyList()) }
    var showDeleted by remember(channelId) { mutableStateOf(false) }
    var title by remember(channelId) { mutableStateOf("") }
    var content by remember(channelId) { mutableStateOf("") }
    var caret by remember(channelId) { mutableIntStateOf(0) }
    var selection by remember(channelId) { mutableStateOf<Int?>(null) }
    var composing by remember(channelId) { mutableStateOf(false) }
    var dismissedSuggestions by remember(channelId) { mutableStateOf(false) }
    var activeSuggestion by remember(channelId) { mutableIntStateOf(0) }
    var editing by remember(channelId) { mutableStateOf(false) }
    var error by remember(channelId) { mutableStateOf("") }
    var busy by remember(channelId) { mutableStateOf(false) }
    var loadInFlight by remember(channelId) { mutableStateOf(false) }
    var latest by remember(channelId) { mutableStateOf<Row?>(null) }
    var latestFetch by remember(channelId) { mutableStateOf(ChannelNoteLatestFetchState.Empty) }
    var taskDraft by remember(channelId) { mutableStateOf<String?>(null) }
    var deleteConfirmation by remember(channelId) { mutableStateOf(ChannelNoteDeleteConfirmation.Empty) }
    var deleteConflict by remember(channelId) { mutableStateOf(false) }
    var tabSwitch by remember(channelId) { mutableStateOf(ChannelNoteTabSwitchState.Empty) }
    var revision by remember(channelId) { mutableIntStateOf(0) }
    val tabBounds = remember(channelId) { mutableStateMapOf<String, Rect>() }
    var dragX by remember(channelId) { mutableFloatStateOf(0f) }
    var dragTarget by remember(channelId) { mutableStateOf<Pair<String, Boolean>?>(null) }
    val scope = rememberCoroutineScope()
    val dirty = (editing && selected != null &&
        (title != selected?.text("title") || content != selected?.text("content"))) || taskDraft != null
    var previousNoteId by remember(channelId) { mutableStateOf(noteId) }
    LaunchedEffect(noteId) {
        if (previousNoteId != noteId) {
            previousNoteId = noteId; editing = false; latest = null; taskDraft = null
            latestFetch = latestFetch.forCurrentNote(noteId); deleteConflict = false
            deleteConfirmation = deleteConfirmation.forCurrentNote(noteId)
            tabSwitch = tabSwitch.forCurrentRoute(noteId, routeGeneration.generation)
        }
    }
    LaunchedEffect(dirty) { app.platform.invoke("noteDirty", dirty.toString()) }
    DisposableEffect(channelId) {
        onDispose { app.scope.launch { app.platform.invoke("noteDirty", "false") } }
    }

    fun errorMessage(e: Exception): String = when ((e as? ApiException)?.status) {
        403 -> app.t("common.errors.forbidden")
        404 -> app.t("channels.notes.notFound")
        409 -> app.t("channels.notes.conflict")
        else -> app.t("channels.notes.loadError")
    }
    fun changeTab(next: String?) {
        if (busy) return
        if (dirty) { tabSwitch = ChannelNoteTabSwitchState.pending(noteId, routeGeneration.generation, next) }
        else {
            editing = false; latest = null; taskDraft = null
            latestFetch = ChannelNoteLatestFetchState.Empty; deleteConflict = false
            deleteConfirmation = ChannelNoteDeleteConfirmation.Empty
            app.navigate(notePath(app, next))
        }
    }
    fun refresh() { revision++ }
    fun isCurrentNoteRoute(startNoteId: String?, startGeneration: Int): Boolean =
        routeGeneration.matches(channelId, startNoteId, startGeneration) &&
            app.channelId == channelId && app.query["note"] == startNoteId
    fun createNote() {
        if (busy) return
        val startNoteId = noteId
        val startGeneration = routeGeneration.generation
        busy = true
        scope.launch {
            try {
                val created = noteData(app.api.request(path, "POST", noteBody(app.t("channels.notes.untitled"), "")))
                if (!isCurrentNoteRoute(startNoteId, startGeneration)) {
                    refresh()
                    return@launch
                }
                editing = false; error = ""; refresh()
                app.navigate(notePath(app, created.text("id")))
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) { if (isCurrentNoteRoute(startNoteId, startGeneration)) error = errorMessage(e) }
            finally { busy = false }
        }
    }
    suspend fun loadLatest(id: String): Row = noteData(app.api.request("$path/$id"))
    fun save(overrideVersion: Int? = null, taskContent: String? = null) {
        val current = selected ?: return
        val startNoteId = current.text("id")
        val startGeneration = routeGeneration.generation
        if (!canApplyChannelNoteResponse(routeGeneration, channelId, startNoteId, startGeneration, selected?.text("id")) ||
            !isCurrentNoteRoute(startNoteId, startGeneration)
        ) return
        when (channelNoteSaveAction(canWrite, busy, hasTaskContent = taskContent != null)) {
            ChannelNoteSaveAction.Reject -> return
            ChannelNoteSaveAction.QueueTaskDraft -> { taskDraft = taskContent; return }
            ChannelNoteSaveAction.Persist -> if (taskContent != null) taskDraft = taskContent
        }
        val nextContent = taskContent ?: content
        busy = true
        scope.launch {
            var success = false
            try {
                val version = overrideVersion ?: current.text("version").toInt()
                val saved = noteData(app.api.request("$path/${current.text("id")}", "PATCH", noteBody(if (taskContent != null) current.text("title") else title, nextContent, version)))
                if (!canApplyChannelNoteResponse(routeGeneration, channelId, startNoteId, startGeneration, selected?.text("id")) ||
                    !isCurrentNoteRoute(startNoteId, startGeneration)
                ) return@launch
                selected = saved
                if (taskContent == null || taskDraft == nextContent) taskDraft = null
                title = saved.text("title")
                content = saved.text("content")
                editing = false
                latest = null
                latestFetch = ChannelNoteLatestFetchState.Empty
                error = ""
                success = true
                refresh()
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (!canApplyChannelNoteResponse(routeGeneration, channelId, startNoteId, startGeneration, selected?.text("id")) ||
                    !isCurrentNoteRoute(startNoteId, startGeneration)
                ) return@launch
                error = errorMessage(e)
                if ((e as? ApiException)?.status == 409) {
                    val id = current.text("id")
                    try {
                        val fetched = loadLatest(id)
                        if (!canApplyChannelNoteResponse(routeGeneration, channelId, startNoteId, startGeneration, selected?.text("id")) ||
                            !isCurrentNoteRoute(startNoteId, startGeneration)
                        ) return@launch
                        latest = fetched
                        latestFetch = ChannelNoteLatestFetchState.Empty.markLoaded(id, forDelete = false)
                    }
                    catch (cancel: CancellationException) { throw cancel }
                    catch (fetchError: Exception) {
                        if (!canApplyChannelNoteResponse(routeGeneration, channelId, startNoteId, startGeneration, selected?.text("id")) ||
                            !isCurrentNoteRoute(startNoteId, startGeneration)
                        ) return@launch
                        latest = null
                        latestFetch = ChannelNoteLatestFetchState.Empty.markFailed(id, errorMessage(fetchError), forDelete = false)
                    }
                }
            } finally {
                busy = false
                if (success && taskDraft != null) save(taskContent = taskDraft)
            }
        }
    }
    LaunchedEffect(channelId, noteId, revision) {
        loadInFlight = true
        try {
            notes = app.api.request("$path?summary=true").rows()
            if (noteId != null && !editing && !busy && taskDraft == null) {
                if (notes.none { it.text("id") == noteId }) {
                    selected = null
                    error = app.t("channels.notes.notFound")
                    return@LaunchedEffect
                }
                val fetched = loadLatest(noteId)
                if (!editing && !busy && taskDraft == null && app.query["note"] == noteId) {
                    selected = fetched
                    title = fetched.text("title")
                    content = fetched.text("content")
                }
            } else if (noteId == null && !editing) selected = null
            if (!editing && !busy && taskDraft == null) error = ""
        } catch (e: CancellationException) { throw e }
        catch (e: Exception) {
            if ((e as? ApiException)?.status == 404 && !editing && !busy && taskDraft == null && app.query["note"] == noteId) selected = null
            error = errorMessage(e)
        }
        finally { loadInFlight = false }
    }
    LaunchedEffect(channelId, noteId) {
        while (isActive) {
            delay(5_000)
            if (shouldPollChannelNoteSummary(loadInFlight, busy)) {
                val startNoteId = noteId
                val startGeneration = routeGeneration.generation
                loadInFlight = true
                try {
                    val summaries = app.api.request("$path?summary=true").rows()
                    if (!isCurrentNoteRoute(startNoteId, startGeneration)) continue
                    // Refresh the tab labels without replacing the active editor or its drafts.
                    notes = summaries
                    if (startNoteId != null) {
                        val summary = summaries.firstOrNull { it.text("id") == startNoteId }
                        if (shouldReportMissingChannelNote(summary != null, editing, taskDraft != null, busy)) {
                            selected = null
                            error = app.t("channels.notes.notFound")
                        } else {
                            val selectedNote = selected?.takeIf { it.text("id") == startNoteId }
                            val selectedVersion = selectedNote?.text("version")?.toIntOrNull()
                            val summaryVersion = summary?.text("version")?.toIntOrNull()
                            if (shouldRefreshChannelNoteDetail(
                                    summaryVersion,
                                    selectedVersion,
                                    editing,
                                    taskDraft != null,
                                    busy,
                                )
                            ) {
                                val fetched = loadLatest(startNoteId)
                                if (canApplyChannelNoteSummaryDetail(
                                        routeGeneration,
                                        channelId,
                                        startNoteId,
                                        startGeneration,
                                        selectedVersion,
                                        selected?.takeIf { it.text("id") == startNoteId }?.text("version")?.toIntOrNull(),
                                        selected?.text("id"),
                                        editing,
                                        taskDraft != null,
                                        busy,
                                    ) && isCurrentNoteRoute(startNoteId, startGeneration)
                                ) {
                                    selected = fetched
                                    title = fetched.text("title")
                                    content = fetched.text("content")
                                    error = ""
                                }
                            }
                        }
                    }
                } catch (e: CancellationException) { throw e }
                catch (_: Exception) { /* Keep the last known list and retry on the next interval. */ }
                finally { loadInFlight = false }
            }
        }
    }
    LaunchedEffect(canWrite) {
        if (!canWrite) deleteConfirmation = ChannelNoteDeleteConfirmation.Empty
        if (!canWrite && taskDraft != null) {
            val next = ChannelNoteDraftState(title, content, editing, taskDraft)
                .onWriteAccessLost(selected?.text("content").orEmpty())
            title = next.title; content = next.content; editing = next.editing; taskDraft = next.taskDraft
            latest = null; latestFetch = ChannelNoteLatestFetchState.Empty; error = ""
        }
    }
    val latestRetryAvailable = latestFetch.canRetry(noteId, selected?.text("id"))
    fun retryLatest() {
        val current = selected ?: return
        val currentId = current.text("id")
        if (busy || !latestFetch.canRetry(noteId, currentId)) return
        val forDelete = latestFetch.forDelete
        val startNoteId = noteId ?: return
        val startGeneration = routeGeneration.generation
        busy = true
        scope.launch {
            try {
                val fetched = loadLatest(currentId)
                if (!canApplyChannelNoteResponse(routeGeneration, channelId, startNoteId, startGeneration, selected?.text("id")) ||
                    !isCurrentNoteRoute(startNoteId, startGeneration)
                ) return@launch
                latest = fetched
                latestFetch = latestFetch.markLoaded(currentId, forDelete)
                error = ""
                if (forDelete) deleteConflict = true
            } catch (cancel: CancellationException) { throw cancel }
            catch (e: Exception) {
                if (!canApplyChannelNoteResponse(routeGeneration, channelId, startNoteId, startGeneration, selected?.text("id")) ||
                    !isCurrentNoteRoute(startNoteId, startGeneration)
                ) return@launch
                latestFetch = latestFetch.markFailed(currentId, errorMessage(e), forDelete)
            } finally { busy = false }
        }
    }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Button(app.t("channels.notes.messages"), enabled = !busy, role = androidx.compose.ui.semantics.Role.Tab, selected = noteId == null, variant = "tab") { changeTab(null) }
            notes.forEachIndexed { index, note ->
                fun reorder(target: Int) {
                    if (target !in notes.indices || target == index || busy) return
                    val updated = notes.toMutableList()
                    val moved = updated.removeAt(index)
                    updated.add(target, moved)
                    busy = true
                    scope.launch {
                        try {
                            val body = buildJsonObject {
                                put("note_ids", JsonArray(updated.map { JsonPrimitive(it.text("id")) }))
                                put("expected_note_ids", JsonArray(notes.map { JsonPrimitive(it.text("id")) }))
                            }
                            app.api.request("$path/order", "PATCH", body)
                            notes = updated; error = ""
                        } catch (e: CancellationException) { throw e }
                        catch (e: Exception) { error = errorMessage(e) }
                        finally { busy = false; refresh() }
                    }
                }
                val noteKey = note.text("id")
                val indicator = dragTarget?.takeIf { it.first == noteKey }
                val indicatorColor = MaterialTheme.colorScheme.primary
                val dragModifier = if (canWrite && wide) Modifier.pointerInput(noteKey, notes) {
                    detectDragGestures(
                        onDragStart = { pos -> dragX = (tabBounds[noteKey]?.left ?: 0f) + pos.x },
                        onDragEnd = {
                            val drop = dragTarget
                            dragTarget = null
                            if (drop != null) {
                                val destination = notes.indexOfFirst { it.text("id") == drop.first }
                                val insertion = destination + if (drop.second) 1 else 0
                                reorder((if (index < insertion) insertion - 1 else insertion).coerceIn(0, notes.lastIndex))
                            }
                        },
                        onDragCancel = { dragTarget = null },
                        onDrag = { change, amount ->
                            change.consume()
                            dragX += amount.x
                            val target = notes.minByOrNull { item ->
                                val bounds = tabBounds[item.text("id")]
                                if (bounds == null) Float.MAX_VALUE else kotlin.math.abs(dragX - bounds.center.x)
                            }
                            val bounds = target?.let { tabBounds[it.text("id")] }
                            if (target != null && bounds != null) dragTarget = target.text("id") to (dragX >= bounds.center.x)
                        },
                    )
                } else Modifier
                Row(Modifier.onGloballyPositioned { tabBounds[noteKey] = it.boundsInRoot() }.then(dragModifier).drawBehind {
                    if (indicator != null) {
                        val x = if (indicator.second) size.width else 0f
                        drawLine(indicatorColor, Offset(x, 0f), Offset(x, size.height), strokeWidth = 2.dp.toPx())
                    }
                }, verticalAlignment = Alignment.CenterVertically) {
                    Button(note.text("title"), enabled = !busy, role = androidx.compose.ui.semantics.Role.Tab, selected = noteId == noteKey, variant = "tab") { changeTab(noteKey) }
                    if (canWrite && !wide) {
                        Button("↑", enabled = index > 0 && !busy, accessibleLabel = "${app.t("channels.notes.moveUp")} ${note.text("title")}") { reorder(index - 1) }
                        Button("↓", enabled = index < notes.lastIndex && !busy, accessibleLabel = "${app.t("channels.notes.moveDown")} ${note.text("title")}") { reorder(index + 1) }
                    }
                }
            }
            if (canWrite) {
                Button("+", enabled = !busy, accessibleLabel = app.t("channels.notes.new")) {
                    if (dirty) { tabSwitch = ChannelNoteTabSwitchState.pending(noteId, routeGeneration.generation, "__create__") }
                    else createNote()
                }
                Button(app.t("channels.notes.deleted"), enabled = !busy) {
                    showDeleted = !showDeleted
                    if (showDeleted) scope.launch {
                        try { deleted = app.api.request("$path/deleted").rows() }
                        catch (e: CancellationException) { throw e }
                        catch (e: Exception) { error = errorMessage(e) }
                    }
                }
            }
        }
        HorizontalDivider()
        if (error.isNotEmpty() && !latestRetryAvailable) Row(Modifier.fillMaxWidth().padding(8.dp)) {
            Text(error, Modifier.weight(1f), color = MaterialTheme.colorScheme.error)
            Button(app.t("channels.notes.retry"), enabled = !busy) { refresh() }
        }
        if (latestRetryAvailable) Row(Modifier.fillMaxWidth().padding(8.dp)) {
            Text(latestFetch.error.ifEmpty { app.t("channels.notes.loadError") }, Modifier.weight(1f), color = MaterialTheme.colorScheme.error)
            Button(app.t("channels.notes.retry"), enabled = !busy) { retryLatest() }
        }
        if (showDeleted && canWrite) deleted.forEach { row ->
            Button("${app.t("channels.notes.restore")}: ${row.text("title")}", enabled = !busy) {
                busy = true
                scope.launch {
                    try { app.api.request("$path/${row.text("id")}/restore", "POST", obj()); deleted = app.api.request("$path/deleted").rows(); error = ""; refresh() }
                    catch (e: CancellationException) { throw e }
                    catch (e: Exception) { error = errorMessage(e) }
                    finally { busy = false }
                }
            }
        }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            Box(if (noteId == null) Modifier.fillMaxSize() else Modifier.size(0.dp).alpha(0f).clearAndSetSemantics { }) { messages() }
            if (noteId != null) {
                val note = selected?.takeIf { it.text("id") == noteId }
                if (note == null) Column(Modifier.padding(16.dp)) {
                    Text(if (error.isNotEmpty()) error else app.t("common.loading"))
                    Button(app.t("channels.notes.messages")) { changeTab(null) }
                } else Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    if (editing) {
                        val range = if (!dismissedSuggestions && !composing) completionAt(content, caret.coerceIn(0, content.length)).takeIf { it?.kind == "mention" } else null
                        val options = completionOptions(range, users, members.filter { it.text("channel_id") == channelId }.map { it.text("user_id") }.toSet())
                        fun choose(option: CompletionOption) {
                            if (range != null) {
                                val (next, nextCaret) = insertCompletion(content, range, option.name)
                                content = next; caret = nextCaret; selection = nextCaret; dismissedSuggestions = true
                            }
                        }
                        val saveShortcut = Modifier.onPreviewKeyEvent {
                            if (!composing && it.type == KeyEventType.KeyDown && it.key == Key.Enter && !it.isShiftPressed && (if (app.env.mac) it.isMetaPressed else it.isCtrlPressed)) {
                                if (!busy && title.isNotBlank()) save()
                                true
                            } else false
                        }
                        val bodyKeys = Modifier.onPreviewKeyEvent {
                            if (composing || it.type != KeyEventType.KeyDown) false
                            else if (options.isNotEmpty() && !it.isCtrlPressed && !it.isMetaPressed && !it.isShiftPressed) {
                                when (it.key) {
                                    Key.DirectionDown -> { activeSuggestion = (activeSuggestion + 1) % options.size; true }
                                    Key.DirectionUp -> { activeSuggestion = (activeSuggestion + options.size - 1) % options.size; true }
                                    Key.Enter, Key.Tab -> { choose(options[activeSuggestion.coerceIn(0, options.lastIndex)]); true }
                                    Key.Escape -> { dismissedSuggestions = true; true }
                                    else -> false
                                }
                            } else if (it.key == Key.Enter && !it.isShiftPressed && (if (app.env.mac) it.isMetaPressed else it.isCtrlPressed)) {
                                if (!busy && title.isNotBlank()) save(); true
                            } else false
                        }
                        Field(title, app.t("channels.notes.noteTitle"), modifier = Modifier.fillMaxWidth().then(saveShortcut), enabled = !busy, onComposition = { composing = it }, maxLength = CHANNEL_NOTE_TITLE_MAX_LENGTH) { title = it }
                        Field(content, app.t("channels.notes.content"), modifier = Modifier.fillMaxWidth().then(bodyKeys), enabled = !busy,
                            singleLine = false, minimumHeight = 220.dp, selection = selection,
                            onSelection = { caret = it.coerceAtLeast(0); selection = null }, onComposition = { composing = it },
                            maxLength = CHANNEL_NOTE_CONTENT_MAX_LENGTH,
                        ) { value -> content = value; dismissedSuggestions = false; activeSuggestion = 0 }
                        options.forEachIndexed { index, option ->
                            Button("@${option.name} ${option.label}", selected = index == activeSuggestion) { choose(option) }
                        }
                        if (dirty) Text(app.t("channels.notes.unsavedChanges"), muted = true)
                        latest?.let { other ->
                            Text("${app.t("channels.notes.latestVersion")}: ${other.text("title")}")
                            ChannelNoteMarkdown(app, other.text("content"), false, false) { }
                            Button(app.t("channels.notes.useLatest"), enabled = !busy) {
                                val next = ChannelNoteDraftState(title, content, editing, taskDraft)
                                    .useLatest(other.text("title"), other.text("content"))
                                selected = other; title = next.title; content = next.content; editing = next.editing; taskDraft = next.taskDraft
                                latest = null; latestFetch = ChannelNoteLatestFetchState.Empty; error = ""
                            }
                            Button(app.t("channels.notes.overwriteLatest"), enabled = canPersistChannelNote(canWrite, busy)) { save(other.text("version").toInt()) }
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(app.t("channels.notes.save"), enabled = canPersistChannelNote(canWrite, busy) && title.isNotBlank(), primary = true) { save() }
                            Button(app.t("common.cancel"), enabled = !busy) {
                                if (!dirty) editing = false
                                else tabSwitch = ChannelNoteTabSwitchState.pending(noteId, routeGeneration.generation, "__cancel__")
                            }
                        }
                    } else {
                        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                            Text(note.text("title"), Modifier.weight(1f))
                            Button(app.t("channels.notes.copyUrl")) { scope.launch { try { app.platform.invoke("copyUrl", notePath(app, noteId)); app.showToast(app.t("channels.notes.copied"), ToastKind.Success) } catch (_: Exception) { app.showToast(app.t("channels.notes.copyError"), ToastKind.Error) } } }
                            if (canWrite) {
                                Button(app.t("channels.notes.edit"), enabled = !busy) { title = note.text("title"); content = taskDraft ?: note.text("content"); editing = true }
                                Button(app.t("channels.notes.delete"), enabled = !busy, destructive = true) {
                                    deleteConfirmation = ChannelNoteDeleteConfirmation.forNote(note.text("id"))
                                }
                            }
                        }
                        if (deleteConflict && latest != null) {
                            Text("${app.t("channels.notes.latestVersion")}: ${latest!!.text("title")}")
                            ChannelNoteMarkdown(app, latest!!.text("content"), false, false) { }
                            Button(app.t("channels.notes.deleteLatest"), enabled = !busy) {
                                val latestNote = latest ?: return@Button
                                selected = latestNote; latest = null; latestFetch = ChannelNoteLatestFetchState.Empty
                                deleteConflict = false; deleteConfirmation = ChannelNoteDeleteConfirmation.forNote(latestNote.text("id"))
                            }
                            Button(app.t("common.cancel")) { latest = null; latestFetch = latestFetch.clear(); deleteConflict = false; error = "" }
                        }
                        ChannelNoteMarkdown(app, taskDraft ?: note.text("content"), canWrite, busy && taskDraft == null) { save(taskContent = it) }
                        if (taskDraft != null) {
                            Text(app.t(if (busy) "channels.notes.saving" else "channels.notes.saveFailed"))
                            if (!busy && canWrite) Button(app.t("channels.notes.retry")) { save(taskContent = taskDraft) }
                            latest?.let { other ->
                                Text("${app.t("channels.notes.latestVersion")}: ${other.text("title")}")
                                ChannelNoteMarkdown(app, other.text("content"), false, false) { }
                                Button(app.t("channels.notes.useLatest"), enabled = !busy) {
                                    val next = ChannelNoteDraftState(title, content, editing, taskDraft)
                                        .useLatest(other.text("title"), other.text("content"))
                                    selected = other; title = next.title; content = next.content; editing = next.editing; taskDraft = next.taskDraft
                                    latest = null; latestFetch = ChannelNoteLatestFetchState.Empty; error = ""
                                }
                                Button(app.t("channels.notes.overwriteLatest"), enabled = canPersistChannelNote(canWrite, busy)) { save(other.text("version").toInt(), taskDraft) }
                            }
                        }
                    }
                }
            }
        }
    }
    val pendingSwitch = tabSwitch.takeIf { it.matches(noteId, routeGeneration.generation) }
    if (pendingSwitch != null) ConfirmationPanel(
        title = app.t("channels.notes.unsaved"), description = "", action = app.t("channels.notes.discardChanges"), busy = false, error = null,
        cancel = { tabSwitch = ChannelNoteTabSwitchState.Empty },
        confirm = {
            val currentSwitch = tabSwitch.takeIf { it.matches(app.query["note"], routeGeneration.generation) }
            tabSwitch = ChannelNoteTabSwitchState.Empty
            if (currentSwitch != null) {
                editing = false; latest = null; taskDraft = null
                latestFetch = ChannelNoteLatestFetchState.Empty; deleteConflict = false
                deleteConfirmation = ChannelNoteDeleteConfirmation.Empty
                if (currentSwitch.targetTab == "__cancel__") { title = selected?.text("title").orEmpty(); content = selected?.text("content").orEmpty() }
                else scope.launch {
                    app.platform.invoke("noteDirty", "false")
                    if (currentSwitch.targetTab == "__create__") createNote() else app.navigate(notePath(app, currentSwitch.targetTab))
                }
            }
        },
    )
    val confirmableDeleteId = deleteConfirmation.confirmableNoteId(noteId, selected?.text("id"))
    if (confirmableDeleteId != null) ConfirmationPanel(
        title = app.t("channels.notes.deleteConfirm"), description = "", action = app.t("channels.notes.delete"), busy = busy, error = null,
        cancel = { deleteConfirmation = ChannelNoteDeleteConfirmation.Empty },
        confirm = {
            val targetId = deleteConfirmation.confirmableNoteId(app.query["note"], selected?.text("id"))
            deleteConfirmation = ChannelNoteDeleteConfirmation.Empty
            val current = selected?.takeIf { it.text("id") == targetId }
            if (current != null && !busy && canWrite) {
                val startNoteId = current.text("id")
                val startGeneration = routeGeneration.generation
                busy = true
                scope.launch {
                    try {
                        app.api.request("$path/${current.text("id")}", "DELETE", buildJsonObject { put("version", current.text("version").toInt()) })
                        if (!isCurrentNoteRoute(startNoteId, startGeneration)) {
                            refresh()
                            return@launch
                        }
                        error = ""; refresh(); app.navigate(notePath(app, null))
                    }
                    catch (e: CancellationException) { throw e }
                    catch (e: Exception) {
                        if (isCurrentNoteRoute(startNoteId, startGeneration)) {
                            error = errorMessage(e)
                        }
                        if (isCurrentNoteRoute(startNoteId, startGeneration) && (e as? ApiException)?.status == 409) {
                            val id = current.text("id")
                            try {
                                val fetched = loadLatest(id)
                                if (!canApplyChannelNoteResponse(routeGeneration, channelId, startNoteId, startGeneration, selected?.text("id")) ||
                                    !isCurrentNoteRoute(startNoteId, startGeneration)
                                ) return@launch
                                latest = fetched
                                latestFetch = ChannelNoteLatestFetchState.Empty.markLoaded(id, forDelete = true)
                                deleteConflict = true
                            }
                            catch (cancel: CancellationException) { throw cancel }
                            catch (fetchError: Exception) {
                                if (!canApplyChannelNoteResponse(routeGeneration, channelId, startNoteId, startGeneration, selected?.text("id")) ||
                                    !isCurrentNoteRoute(startNoteId, startGeneration)
                                ) return@launch
                                latest = null
                                latestFetch = ChannelNoteLatestFetchState.Empty.markFailed(id, errorMessage(fetchError), forDelete = true)
                            }
                        }
                    }
                    finally { busy = false }
                }
            }
        },
    )
}
