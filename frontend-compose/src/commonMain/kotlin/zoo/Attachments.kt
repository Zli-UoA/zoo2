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
import androidx.compose.ui.unit.sp
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

private val attachmentExtensions =
    mapOf(
        "png" to "image/png",
        "jpg" to "image/jpeg",
        "jpeg" to "image/jpeg",
        "webp" to "image/webp",
        "gif" to "image/gif",
        "pdf" to "application/pdf",
        "txt" to "text/plain",
        "md" to "text/markdown",
        "csv" to "text/csv",
        "json" to "application/json",
        "xml" to "application/xml",
        "zip" to "application/zip",
        "gz" to "application/gzip",
        "tar" to "application/x-tar",
        "7z" to "application/x-7z-compressed",
        "rar" to "application/vnd.rar",
        "doc" to "application/msword",
        "docx" to "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" to "application/vnd.ms-excel",
        "xlsx" to "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" to "application/vnd.ms-powerpoint",
        "pptx" to "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" to "audio/mpeg",
        "m4a" to "audio/mp4",
        "wav" to "audio/wav",
        "ogg" to "audio/ogg",
        "flac" to "audio/flac",
        "mp4" to "video/mp4",
        "webm" to "video/webm",
        "mov" to "video/quicktime",
    )
private val forbiddenAttachmentExtensions =
    setOf("exe", "com", "scr", "msi", "dll", "so", "dylib", "bat", "cmd", "sh")
private val forbiddenAttachmentTypes =
    setOf(
        "application/x-msdownload",
        "application/x-executable",
        "application/x-dosexec",
        "application/x-msdos-program",
        "application/x-sharedlib",
        "application/x-batch",
        "application/x-sh",
    )

fun attachmentType(name: String, declared: String): String? {
    val extension = name.substringAfterLast('.').lowercase()
    if (extension in forbiddenAttachmentExtensions) return null
    val mime = declared.substringBefore(';').trim().lowercase()
    if (
        mime in setOf("application/octet-stream", "binary/octet-stream") ||
            !Regex("^[a-z0-9!#$&^_.+-]+/[a-z0-9!#$&^_.+-]+$").matches(mime)
    )
        return attachmentExtensions[extension] ?: "application/octet-stream"
    return mime.takeUnless { it in forbiddenAttachmentTypes }
}

fun attachmentRejection(file: Row, count: Int): String? =
    when {
        count >= MAX_ATTACHMENTS -> "tooMany"
        (file.text("size").toLongOrNull() ?: 0) <= 0 -> "empty"
        (file.text("size").toLongOrNull() ?: 0) > MAX_ATTACHMENT_BYTES -> "tooLarge"
        attachmentType(file.text("name"), file.text("type")) == null -> "unsupported"
        else -> null
    }

fun attachmentName(name: String, type: String, fallback: String): String {
    val mime = type.substringBefore(';').trim().lowercase()
    val extension =
        attachmentExtensions.entries.firstOrNull { it.value == mime }?.key
            ?: mime.substringAfter('/', "").replace(Regex("[^a-z0-9]+"), "").take(12).ifEmpty {
                "bin"
            }
    val cleaned =
        clampCodePoints(
            name.map { if (it.code < 32 || it.code == 127) '_' else it }.joinToString("").trim(),
            255,
        )
    if (cleaned.isEmpty()) return "attachment-$fallback.$extension"
    if (Regex("\\.[^./\\\\]+$").containsMatchIn(cleaned)) return cleaned
    return "${clampCodePoints(cleaned, 254 - extension.length)}.$extension"
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

    fun download(file: Row) {
        app.run {
            try {
                app.platform.invoke(
                    "attachmentDownload",
                    obj("url" to get(file.text("id")), "name" to file.text("file_name")).toString(),
                )
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                app.toast = app.t("attachments.downloadError")
            }
        }
    }
}

class AttachmentDraft(val file: Row, val name: String, val type: String) {
    var uploaded by mutableStateOf<Row?>(null)
    var bitmap by mutableStateOf<ImageBitmap?>(null)
    var previewJob: Job? = null
}

class AttachmentDrafts(
    private val app: Session,
    val channel: String,
    parentScope: CoroutineScope,
    private val upload: (suspend (Row) -> Row)? = null,
) {
    private val job = SupervisorJob(parentScope.coroutineContext[Job])
    private val scope = CoroutineScope(parentScope.coroutineContext + job)
    val owner = app.platform.uuid()
    val items = mutableStateListOf<AttachmentDraft>()
    private val owned = mutableMapOf<String, AttachmentDraft>()
    var error by mutableStateOf<String?>(null)
    var enabled = false
    var acceptFiles = true
    private var bounds: Row = JsonObject(emptyMap())

    fun updateBounds(left: Float, top: Float, width: Float, height: Float) {
        val next = obj("left" to left, "top" to top, "width" to width, "height" to height)
        if (bounds != next) {
            bounds = next
            bind()
        }
    }

    fun bind() =
        app.platform.fileTarget(
            owner,
            buildJsonObject {
                    put("enabled", enabled && acceptFiles)
                    put("bounds", bounds)
                }
                .toString(),
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
            if (
                !enabled ||
                    !acceptFiles ||
                    data.text("origin") == "paste" && (mime == null || !seen.add(key))
            ) {
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
            if (mime in imageTypes)
                draft.previewJob =
                    scope.launch {
                        try {
                            val bytes =
                                Base64.decode(app.platform.invoke("fileRead", file.text("id")))
                            app.platform.imageFrames(bytes).collect { draft.bitmap = it }
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {}
                    }
            scope.launch {
                try {
                    if (upload != null) {
                        val result = upload(file)
                        if (draft in items) draft.uploaded = result
                    } else {
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
                        if (draft in items)
                            draft.uploaded = obj("id" to presign.text("attachment_id"))
                    }
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
                            put("accept", "")
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
    upload: (suspend (Row) -> Row)? = null,
): AttachmentDrafts {
    val scope = rememberCoroutineScope()
    val drafts = remember(key) { AttachmentDrafts(app, channel, scope, upload) }
    drafts.enabled = enabled && app.dialog == null && app.overlays.none { it.visible }
    SideEffect { drafts.bind() }
    DisposableEffect(drafts) { onDispose { drafts.close() } }
    return drafts
}

@Composable
fun DraftAttachments(app: Session, drafts: AttachmentDrafts) {
    if (drafts.items.isNotEmpty())
        FlowRow(
            modifier = Modifier.padding(bottom = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            drafts.items.forEach { draft ->
                Box(
                    (if (draft.type in imageTypes) Modifier.size(80.dp)
                        else Modifier.width(224.dp).height(80.dp))
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
                    if (draft.type !in imageTypes)
                        Row(
                            Modifier.padding(start = 8.dp, end = 36.dp)
                                .align(Alignment.CenterStart),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Icon("paperclip", modifier = Modifier.size(20.dp))
                            Column {
                                Text(
                                    draft.name,
                                    small = true,
                                    maxLines = 1,
                                    weight = androidx.compose.ui.text.font.FontWeight.Medium,
                                )
                                Text(
                                    attachmentSize(draft.file.text("size")),
                                    size = 11.sp,
                                    muted = true,
                                )
                            }
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
            Modifier.padding(bottom = 8.dp, start = 4.dp)
                .then(webControlModifier("alert"))
                .semantics { liveRegion = LiveRegionMode.Assertive },
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
    var preview by remember(id) { mutableStateOf(false) }
    if (preview) AttachmentPreview(app, file) { preview = false }
    val remoteFailure = app.attachmentLinks.errors[id] == true
    if (image && bitmap != null && !loading && !remoteFailure)
        Box(
            modifier
                .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(8.dp))
                .clickable(role = Role.Button) { preview = true }
                .semantics {
                    contentDescription =
                        app.t("attachments.preview", "name" to file.text("file_name"))
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
                    app.attachmentLinks.download(file)
                }
        }
}

@Composable
private fun AttachmentPreview(app: Session, file: Row, close: () -> Unit) {
    val id = file.text("id")
    var attempt by remember(id) { mutableStateOf(0) }
    var bitmap by remember(id) { mutableStateOf<ImageBitmap?>(null) }
    var failed by remember(id) { mutableStateOf(false) }
    LaunchedEffect(id, attempt) {
        bitmap = null
        failed = false
        try {
            var url = app.attachmentLinks.get(id, attempt > 0)
            try {
                app.platform.imageFrames(app.image(url)).collect { bitmap = it }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                url = app.attachmentLinks.get(id, true)
                app.platform.imageFrames(app.image(url)).collect { bitmap = it }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            failed = true
        }
    }
    Panel(file.text("file_name"), close, 1280.dp) {
        if (failed) {
            Text(
                app.t("attachments.loadError"),
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Assertive },
            )
            Button(app.t("attachments.retry")) { attempt++ }
        } else if (bitmap == null) Text(app.t("attachments.loading"))
        else
            BoxWithConstraints(Modifier.fillMaxWidth().weight(1f, fill = false)) {
                Image(
                    bitmap!!,
                    file.text("file_name"),
                    Modifier.fillMaxWidth().heightIn(max = maxHeight),
                    contentScale = ContentScale.Fit,
                )
            }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(file.text("file_name"), Modifier.weight(1f), small = true)
            Button(
                app.t("attachments.downloadShort"),
                accessibleLabel = app.t("attachments.download", "name" to file.text("file_name")),
            ) {
                app.attachmentLinks.download(file)
            }
        }
    }
}
