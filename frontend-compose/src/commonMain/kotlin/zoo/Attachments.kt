@file:OptIn(
    kotlin.io.encoding.ExperimentalEncodingApi::class,
    androidx.compose.foundation.layout.ExperimentalLayoutApi::class,
)

package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import io.ktor.http.*
import kotlin.io.encoding.Base64
import kotlin.math.round
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import kotlinx.serialization.json.*

const val MAX_ATTACHMENTS = 10
const val MAX_ATTACHMENT_BYTES = 50L * 1024 * 1024
val imageTypes = listOf("image/png", "image/jpeg", "image/webp", "image/gif")
val LocalAttachments = staticCompositionLocalOf<List<Row>> { emptyList() }

private fun clampCodePoints(value: String, max: Int): String {
    var index = 0
    var count = 0
    while (index < value.length && count++ < max) {
        val ch = value[index++]
        if (ch.isHighSurrogate() && index < value.length && value[index].isLowSurrogate()) index++
    }
    return value.take(index)
}

fun attachmentType(name: String, declared: String): String? =
    declared.substringBefore(';').trim().lowercase().takeIf { it in imageTypes }
        ?: mapOf(
            "png" to "image/png",
            "jpg" to "image/jpeg",
            "jpeg" to "image/jpeg",
            "webp" to "image/webp",
            "gif" to "image/gif",
        )[name.substringAfterLast('.').lowercase()]

fun attachmentRejection(file: Row, count: Int): String? =
    when {
        count >= MAX_ATTACHMENTS -> "tooMany"
        (file.text("size").toLongOrNull() ?: 0) > MAX_ATTACHMENT_BYTES -> "tooLarge"
        attachmentType(file.text("name"), file.text("type")) == null -> "unsupported"
        else -> null
    }

fun attachmentName(name: String, type: String, fallback: String): String =
    clampCodePoints(
            name.map { if (it.code < 32 || it.code == 127) '_' else it }.joinToString("").trim(),
            255,
        )
        .ifEmpty {
            "pasted-image-$fallback.${if (type == "image/jpeg") "jpg" else type.substringAfter('/')}"
        }

fun attachmentSize(value: String): String {
    var size = value.toDoubleOrNull()?.takeIf { it.isFinite() && it >= 0 } ?: return "—"
    val units = listOf("B", "KB", "MB", "GB")
    var unit = 0
    while (size >= 1024 && unit < units.lastIndex) {
        size /= 1024
        unit++
    }
    val number =
        if (unit == 0) size.toLong().toString()
        else (round(size * 10) / 10).toString().removeSuffix(".0")
    return "$number ${units[unit]}"
}

fun attachmentExpiry(url: String, now: Long): Long {
    val params = publicUrl(url)?.parameters ?: return now + 240000
    val seconds =
        params["X-Amz-Expires"]?.toDoubleOrNull()?.takeIf { it.isFinite() && it > 0 }
            ?: return now + 240000
    val issued =
        params["X-Amz-Date"]
            ?.replace(
                Regex("^(\\d{4})(\\d{2})(\\d{2})T(\\d{2})(\\d{2})(\\d{2})Z$"),
                "$1-$2-$3T$4:$5:$6Z",
            )
            ?.let(::timestampMillis) ?: now
    return issued + (seconds * 1000).toLong()
}

fun attachmentPayload(value: JsonElement): Row =
    value.jsonObject.let { (it["data"] as? JsonObject) ?: it }

class AttachmentLinks(private val app: Session) {
    val errors = mutableStateMapOf<String, Boolean>()
    private val cache = mutableMapOf<String, Pair<String, Long>>()
    private val requests = mutableMapOf<String, Deferred<Result<String>>>()

    suspend fun get(id: String, force: Boolean = false): String {
        errors.remove(id)
        val now = timestampMillis(app.platform.now()) ?: 0L
        cache[id]
            ?.takeIf { !force && it.second - now > 5000 }
            ?.let {
                return it.first
            }
        val work =
            requests.getOrPut(id) {
                app.scope.async {
                    try {
                        val url =
                            attachmentPayload(
                                    app.api.request(
                                        "/api/attachments/${id.encodeURLParameter()}/url"
                                    )
                                )
                                .text("download_url")
                        require(publicUrl(url) != null)
                        if (cache.size >= 200) cache.remove(cache.keys.first())
                        cache[id] = url to attachmentExpiry(url, now)
                        Result.success(url)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Result.failure(e)
                    }
                }
            }
        work.invokeOnCompletion { if (requests[id] === work) requests.remove(id) }
        return try {
            work.await().getOrThrow()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            if (errors.size >= 200) errors.remove(errors.keys.first())
            errors[id] = true
            throw e
        } finally {
            if (work.isCompleted && requests[id] === work) requests.remove(id)
        }
    }

    fun open(id: String) {
        val popup = app.platform.uuid()
        app.platform.action("attachmentReserve:$popup")
        app.run {
            val url =
                try {
                    get(id)
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    ""
                }
            app.platform.invoke("attachmentOpen", obj("id" to popup, "url" to url).toString())
        }
    }
}

class AttachmentDraft(val file: Row, val name: String, val type: String) {
    var uploaded by mutableStateOf<Row?>(null)
    var bitmap by mutableStateOf<ImageBitmap?>(null)
    var previewJob: Job? = null
}

class AttachmentDrafts(private val app: Session, val channel: String, parentScope: CoroutineScope) {
    private val job = SupervisorJob(parentScope.coroutineContext[Job])
    private val scope = CoroutineScope(parentScope.coroutineContext + job)
    val owner = app.platform.uuid()
    val items = mutableStateListOf<AttachmentDraft>()
    private val owned = mutableMapOf<String, AttachmentDraft>()
    var error by mutableStateOf<String?>(null)
    var enabled = false

    fun bind() =
        app.platform.fileTarget(
            owner,
            buildJsonObject { put("enabled", enabled) }.toString(),
            ::add,
        )

    fun add(raw: String): Boolean {
        val data = json.parseToJsonElement(raw).jsonObject
        val candidates = (data["files"] as? JsonArray)?.map { it.jsonObject }.orEmpty()
        val seen = mutableSetOf<String>()
        var batchError: String? = null
        var handled = data.text("origin") != "paste" && candidates.isNotEmpty()
        candidates.forEach { file ->
            val key = "${file.text("name")}:${file.text("size")}:${file.text("type")}"
            val mime = attachmentType(file.text("name"), file.text("type"))
            if (!enabled || data.text("origin") == "paste" && (mime == null || !seen.add(key))) {
                app.platform.action("fileRelease:${file.text("id")}")
                return@forEach
            }
            handled = true
            val rejected = attachmentRejection(file, items.size)
            if (rejected != null) {
                if (batchError == null)
                    batchError =
                        app.t(
                            "attachments.$rejected",
                            "max" to if (rejected == "tooMany") "10" else "50",
                        )
                app.platform.action("fileRelease:${file.text("id")}")
                return@forEach
            }
            val draft =
                AttachmentDraft(
                    file,
                    attachmentName(
                        file.text("name"),
                        mime!!,
                        "${timestampMillis(app.platform.now())}-${app.platform.uuid().take(6)}",
                    ),
                    mime,
                )
            items += draft
            owned[file.text("id")] = draft
            draft.previewJob =
                scope.launch {
                    try {
                        val bytes = Base64.decode(app.platform.invoke("fileRead", file.text("id")))
                        app.platform.imageFrames(bytes).collect { draft.bitmap = it }
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {}
                }
            scope.launch {
                try {
                    val presign =
                        attachmentPayload(
                            app.api.request(
                                "/api/attachments/presign",
                                "POST",
                                buildJsonObject {
                                    put("channel_id", channel)
                                    put("file_name", draft.name)
                                    put("file_size", file.text("size").toLong())
                                    put("content_type", mime)
                                },
                            )
                        )
                    if (draft !in items) return@launch
                    require(presign.text("attachment_id").isNotEmpty())
                    app.platform.invoke(
                        "filePut",
                        obj(
                                "id" to file.text("id"),
                                "url" to presign.text("upload_url"),
                                "type" to mime,
                            )
                            .toString(),
                    )
                    if (draft in items) draft.uploaded = obj("id" to presign.text("attachment_id"))
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    if (draft in items) {
                        remove(draft)
                        error = app.t("attachments.uploadError")
                    }
                }
            }
        }
        if (handled) error = batchError
        return handled
    }

    fun pick() {
        scope.launch {
            add(
                app.platform.invoke(
                    "pickFiles",
                    buildJsonObject {
                            put("owner", owner)
                            put("accept", imageTypes.joinToString(","))
                            put("multiple", true)
                        }
                        .toString(),
                )
            )
        }
    }

    fun remove(item: AttachmentDraft) {
        items.remove(item)
        release(listOf(item))
    }

    fun release(values: List<AttachmentDraft>) {
        values.forEach {
            it.previewJob?.cancel()
            it.bitmap = null
            owned.remove(it.file.text("id"))
            app.platform.action("fileRelease:${it.file.text("id")}")
        }
    }

    fun close() {
        job.cancel()
        app.platform.forgetFileTarget(owner)
        release(owned.values.toList())
        items.clear()
    }

    fun canSend(value: String) =
        enabled && items.none { it.uploaded == null } && (value.isNotBlank() || items.isNotEmpty())
}

@Composable
fun rememberAttachmentDrafts(
    app: Session,
    channel: String,
    key: String,
    enabled: Boolean,
): AttachmentDrafts {
    val scope = rememberCoroutineScope()
    val drafts = remember(key) { AttachmentDrafts(app, channel, scope) }
    drafts.enabled = enabled && app.dialog == null && app.overlays.isEmpty()
    SideEffect { drafts.bind() }
    DisposableEffect(drafts) { onDispose { drafts.close() } }
    return drafts
}

@Composable
fun DraftAttachments(app: Session, drafts: AttachmentDrafts) {
    if (drafts.items.isNotEmpty())
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            drafts.items.forEach { draft ->
                Box(
                    Modifier.size(80.dp)
                        .background(
                            MaterialTheme.colorScheme.surfaceVariant,
                            RoundedCornerShape(8.dp),
                        )
                        .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
                ) {
                    draft.bitmap?.let {
                        Image(
                            it,
                            draft.name,
                            Modifier.fillMaxSize(),
                            contentScale = ContentScale.Crop,
                        )
                    }
                    if (draft.uploaded == null)
                        Text(
                            app.t("attachments.uploading"),
                            Modifier.align(Alignment.Center),
                            small = true,
                        )
                    Box(Modifier.align(Alignment.TopEnd)) {
                        IconButton(
                            "x",
                            app.t("attachments.remove", "name" to draft.name),
                            size = 24.dp,
                        ) {
                            drafts.remove(draft)
                        }
                    }
                }
            }
        }
    drafts.error?.let {
        Text(
            it,
            Modifier.semantics { liveRegion = LiveRegionMode.Assertive },
            small = true,
            color = MaterialTheme.colorScheme.error,
        )
    }
}

@Composable
fun MessageAttachments(app: Session, values: List<Row>) {
    if (values.isEmpty()) return
    BoxWithConstraints(Modifier.widthIn(max = 576.dp).fillMaxWidth().padding(top = 8.dp)) {
        val images =
            values.filter {
                it.text("content_type").substringBefore(';').trim().lowercase() in imageTypes
            }
        val files = values - images.toSet()
        val availableWidth = maxWidth
        val square = (minOf(availableWidth, 384.dp) - 8.dp) / 2
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (images.isNotEmpty())
                FlowRow(
                    maxItemsInEachRow = 2,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    images.forEach {
                        AttachmentItem(
                            app,
                            it,
                            if (images.size > 1) Modifier.size(square)
                            else Modifier.widthIn(max = availableWidth).heightIn(max = 320.dp),
                            true,
                            images.size > 1,
                        )
                    }
                }
            files.forEach { AttachmentItem(app, it, Modifier.fillMaxWidth(), false) }
        }
    }
}

@Composable
private fun AttachmentItem(
    app: Session,
    file: Row,
    modifier: Modifier,
    image: Boolean,
    multiple: Boolean = false,
) {
    val id = file.text("id")
    var attempt by remember(id) { mutableStateOf(0) }
    var url by remember(id) { mutableStateOf<String?>(null) }
    var bitmap by remember(id) { mutableStateOf<ImageBitmap?>(null) }
    var loading by remember(id) { mutableStateOf(true) }
    var failed by remember(id) { mutableStateOf(false) }
    LaunchedEffect(id, attempt) {
        loading = true
        failed = false
        try {
            url = app.attachmentLinks.get(id, attempt > 0)
            if (image) {
                try {
                    app.platform.imageFrames(app.image(url!!)).collect {
                        bitmap = it
                        loading = false
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    url = app.attachmentLinks.get(id, true)
                    app.platform.imageFrames(app.image(url!!)).collect {
                        bitmap = it
                        loading = false
                    }
                }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            failed = url == null
        } finally {
            loading = false
        }
    }
    val remoteFailure = app.attachmentLinks.errors[id] == true
    if (image && bitmap != null && !loading && !remoteFailure)
        Box(
            modifier
                .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(8.dp))
                .clickable(role = Role.Button) { app.attachmentLinks.open(id) }
                .semantics {
                    contentDescription = app.t("attachments.open", "name" to file.text("file_name"))
                }
        ) {
            Image(
                bitmap!!,
                null,
                if (multiple) Modifier.fillMaxSize()
                else Modifier.widthIn(max = 576.dp).heightIn(max = 320.dp),
                contentScale = ContentScale.Fit,
            )
        }
    else if (image && loading)
        Box(
            modifier
                .defaultMinSize(minWidth = 128.dp, minHeight = 128.dp)
                .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(8.dp))
        ) {
            Text(app.t("attachments.loading"), Modifier.align(Alignment.Center), small = true)
        }
    else
        Row(
            Modifier.fillMaxWidth()
                .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
                .padding(10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon("paperclip")
            Column(Modifier.weight(1f)) {
                Text(file.text("file_name"), weight = FontWeight.Medium, maxLines = 1)
                Text(attachmentSize(file.text("file_size")), small = true, muted = true)
                if (loading || failed || remoteFailure)
                    Text(
                        app.t(if (loading) "attachments.loading" else "attachments.loadError"),
                        small = true,
                    )
            }
            if (failed || remoteFailure) Button(app.t("attachments.retry")) { attempt++ }
            if (url != null && !remoteFailure)
                IconButton(
                    "download",
                    app.t("attachments.download", "name" to file.text("file_name")),
                ) {
                    app.attachmentLinks.open(id)
                }
        }
}
