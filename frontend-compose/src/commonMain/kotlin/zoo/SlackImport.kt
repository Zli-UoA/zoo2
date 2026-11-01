package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

/** ワーカーが処理中で、結果を待つ必要がある状態 (React の `SLACK_IMPORT_RUNNING`) */
val slackImportRunning = setOf("uploading", "analyzing", "importing", "fetching_files")

/** エクスポート (アップロードした ZIP) か。取り込みのジョブは `source_id` を持つ (React の `isSlackExport`) */
fun isSlackExport(job: Row): Boolean = job.text("source_id").isEmpty()

/**
 * カードで操作できるように出すものと、履歴に回すものに分ける (React の `splitSlackImports`)。
 * 保存中・保存済みのエクスポートと、進行中・失敗した取り込みはすべてカードにし、
 * 進行中の取り込みがなければ最新の完了した取り込みもカードにする。
 */
fun splitSlackImports(imports: List<Row>): Pair<List<Row>, List<Row>> {
    val finished = setOf("completed", "cancelled")
    val exports = imports.filter { isSlackExport(it) && it.text("status") !in finished }
    val runs = imports.filter { !isSlackExport(it) && it.text("status") !in finished }
    val latest = imports.firstOrNull { !isSlackExport(it) && it.text("status") == "completed" }
    val cards = exports + runs.ifEmpty { listOfNotNull(latest) }
    return cards to imports.filter { job -> cards.none { it === job } }
}

/** 「添付ファイルを取得」でやり直せる件数 (React の `retryableFiles`) */
fun slackImportRetryableFiles(job: Row): Int =
    if (job.text("status") == "completed")
        job.text("pending_files").toIntOrNull().orZero() +
            job.text("failed_files").toIntOrNull().orZero()
    else 0

private fun Int?.orZero() = this ?: 0

/** 保存した ZIP から取り込むチャンネルは管理者が選ぶので、初めはどれも取り込まない (React の `defaultMappings`) */
fun slackImportDefaultMappings(channels: List<Row>): List<Row> =
    channels.map { obj("slack_channel_id" to it.text("slack_channel_id"), "action" to "skip") }

/**
 * 取り込み方を選んだときの対応付け (React の `mappingFor`)。以前取り込んだチャンネルは同じ取り込み先に
 * 固定する (メッセージ ID が変わらないため)。`targets` は選べる既存チャンネル。
 */
fun slackImportMappingFor(channel: Row, action: String, targets: List<Row>): Row {
    val id = channel.text("slack_channel_id")
    val existing = channel["existing_channel"] as? JsonObject
    return when (action) {
        "skip" -> obj("slack_channel_id" to id, "action" to "skip")
        "existing" ->
            obj(
                "slack_channel_id" to id,
                "action" to "existing",
                "channel_id" to
                    (existing?.text("id") ?: targets.firstOrNull()?.text("id") ?: ""),
            )
        else ->
            obj(
                "slack_channel_id" to id,
                "action" to "create",
                "name" to channel.text("suggested_name"),
                "is_private" to channel.flag("is_private"),
            )
    }
}

/** 取り込みのジョブは、どの ZIP からの取り込みかを名前に出す */
private fun slackImportTitle(app: Session, job: Row): String =
    if (isSlackExport(job)) job.text("file_name")
    else app.t("slackImport.runTitle", "name" to job.text("file_name"))

/** 失敗の理由。React の `describeImportError` と同じく、403 と `{error}` を見分ける */
data class SlackImportFailure(val status: Int?, val message: String?)

fun slackImportFailure(cause: Throwable): SlackImportFailure =
    (cause as? ApiException)?.let {
        SlackImportFailure(it.status, it.body?.text("error")?.takeIf(String::isNotEmpty))
    } ?: SlackImportFailure(null, null)

fun describeSlackImportFailure(app: Session, failure: SlackImportFailure): String =
    when {
        failure.status == 403 -> app.t("slackImport.forbidden")
        failure.message != null -> failure.message
        else -> app.t("slackImport.failed")
    }

/** 管理者による Slack エクスポートの取り込み (#295)。React の `SlackImportPanel` に対応する。 */
class SlackImportController(
    parent: CoroutineScope,
    private val request: suspend (path: String, method: String, body: JsonElement?) -> JsonElement,
) {
    private val lifetime = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + lifetime)
    private var generation = 0

    var imports by mutableStateOf<List<Row>?>(null)
        private set

    var failure by mutableStateOf<SlackImportFailure?>(null)
        private set

    /** アップロード中なら null 以外 (進み具合は取れないので 0) */
    var uploading by mutableStateOf<Float?>(null)
        private set

    var pending by mutableStateOf(false)
        private set

    val running
        get() = imports?.any { it.text("status") in slackImportRunning } == true

    /** カードで操作できるように出す取り込み (未完了のものすべて) */
    val cards
        get() = splitSlackImports(imports.orEmpty()).first

    val history
        get() = splitSlackImports(imports.orEmpty()).second

    /** 解析が終わるまでは次の ZIP を受け付けない (解析が終わると古い ZIP を置き換える) */
    val busy
        get() =
            cards.any {
                isSlackExport(it) && it.text("status").let { s -> s == "uploading" || s == "analyzing" }
            }

    /** 取り込みを始めるたびに進める。対応付けのフォームを戻し、続けて別のチャンネルを選べるようにする */
    var started by mutableStateOf(0)
        private set

    fun load() {
        if (!lifetime.isActive) return
        val version = ++generation
        scope.launch {
            try {
                val value = request("/api/admin/slack-imports", "GET", null)
                if (version == generation) imports = value.jsonObject.array("imports").rows()
            } catch (cause: CancellationException) {
                throw cause
            } catch (cause: Exception) {
                if (version == generation) failure = slackImportFailure(cause)
            }
        }
    }

    private fun replace(job: Row) {
        generation++
        imports = listOf(job) + imports.orEmpty().filter { it.text("id") != job.text("id") }
    }

    /** ジョブを作って ZIP を置き、解析を依頼する。`put` は presigned URL への PUT。 */
    suspend fun upload(name: String, size: Long, put: suspend (url: String) -> Unit) {
        if (uploading != null || busy) return
        failure = null
        uploading = 0f
        try {
            val created =
                request(
                        "/api/admin/slack-imports",
                        "POST",
                        buildJsonObject {
                            put("file_name", name)
                            put("file_size", size)
                        },
                    )
                    .jsonObject
            put(created.text("upload_url"))
            val id = created.nested("import").text("id")
            replace(request("/api/admin/slack-imports/$id/uploaded", "POST", null).jsonObject)
        } catch (cause: CancellationException) {
            throw cause
        } catch (cause: Exception) {
            failure = slackImportFailure(cause)
            load()
        } finally {
            uploading = null
        }
    }

    private fun act(path: String, method: String, body: JsonElement? = null) {
        if (pending || !lifetime.isActive) return
        pending = true
        scope.launch {
            try {
                replace(request(path, method, body).jsonObject)
            } catch (cause: CancellationException) {
                throw cause
            } catch (cause: Exception) {
                failure = slackImportFailure(cause)
                // 多くは別の画面で先に状態が変わったため (409) なので、一覧を取り直して今の状態を見せる
                load()
            } finally {
                pending = false
            }
        }
    }

    /** 保存した ZIP から取り込みを始める。エクスポートは残り、取り込みのジョブが増える */
    fun start(id: String, mappings: List<Row>) {
        if (pending || !lifetime.isActive) return
        pending = true
        scope.launch {
            try {
                replace(
                    request(
                            "/api/admin/slack-imports/$id/start",
                            "POST",
                            obj("mappings" to JsonArray(mappings)),
                        )
                        .jsonObject
                )
                started++
                // 以前の取り込み先などの表示を新しくする
                load()
            } catch (cause: CancellationException) {
                throw cause
            } catch (cause: Exception) {
                failure = slackImportFailure(cause)
                load()
            } finally {
                pending = false
            }
        }
    }

    fun cancel(id: String) = act("/api/admin/slack-imports/$id", "DELETE")

    fun fetchFiles(id: String) = act("/api/admin/slack-imports/$id/fetch-files", "POST")

    fun close() {
        generation++
        lifetime.cancel()
    }
}

@Composable
fun SlackImportPanel(app: Session) {
    val scope = rememberCoroutineScope()
    val state =
        remember(app) {
            SlackImportController(scope) { path, method, body ->
                app.api.request(path, method, body)
            }
        }
    LaunchedEffect(state) { state.load() }
    DisposableEffect(state) { onDispose { state.close() } }
    // 処理中のジョブがある間は 2 秒ごとに読み直す
    LaunchedEffect(state, state.running) {
        while (state.running) {
            delay(2_000)
            state.load()
        }
    }
    val owner = remember { "slack-import-${app.platform.uuid()}" }
    fun pick() {
        scope.launch {
            val picked =
                runCatching {
                        Json.parseToJsonElement(
                                app.platform.invoke(
                                    "pickFiles",
                                    buildJsonObject {
                                            put("owner", owner)
                                            put("accept", ".zip,application/zip")
                                            put("multiple", false)
                                        }
                                        .toString(),
                                )
                            )
                            .jsonObject
                            .array("files")
                            .rows()
                            .firstOrNull()
                    }
                    .getOrNull() ?: return@launch
            val id = picked.text("id")
            try {
                state.upload(picked.text("name"), picked.text("size").toLongOrNull() ?: 0L) { url
                    ->
                    app.platform.invoke(
                        "filePut",
                        obj("id" to id, "url" to url, "type" to "application/zip").toString(),
                    )
                }
            } finally {
                app.platform.action("fileRelease:$id")
            }
        }
    }

    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        SettingsSection(app.t("slackImport.title"), app.t("slackImport.description")) {
            Button(
                app.t("slackImport.chooseFile"),
                variant = "outline",
                icon = "upload",
                enabled = state.uploading == null && !state.busy,
            ) {
                pick()
            }
            state.uploading?.let { SlackImportProgress(app.t("slackImport.uploading"), it) }
            state.failure?.let {
                Text(
                    describeSlackImportFailure(app, it),
                    Modifier.then(webControlModifier("alert")),
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }
        if (state.imports == null && state.failure == null)
            Text(
                app.t("slackImport.loading"),
                Modifier.then(webControlModifier("status")).semantics {
                    liveRegion = LiveRegionMode.Polite
                },
                muted = true,
            )
        state.cards.forEach { job -> key(job.text("id")) { SlackImportCard(app, state, job) } }
        val history = state.history
        if (history.isNotEmpty())
            SettingsSection(app.t("slackImport.history")) {
                Column(Modifier.fillMaxWidth().then(webControlModifier("list"))) {
                    history.forEachIndexed { index, job ->
                        if (index > 0) HorizontalDivider()
                        Column(
                            Modifier.fillMaxWidth()
                                .then(webControlModifier("listitem"))
                                .padding(vertical = 8.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            CssFlowRow(Modifier.fillMaxWidth()) {
                                Text(slackImportTitle(app, job), small = true)
                                Text(
                                    "${app.t("slackImport.status.${job.text("status")}")} · " +
                                        app.platform.fullTimestamp(job.text("created_at")),
                                    small = true,
                                    muted = true,
                                )
                            }
                            if (slackImportRetryableFiles(job) > 0)
                                SlackImportFileSummary(app, state, job)
                        }
                    }
                }
            }
    }
}

@Composable
private fun SlackImportCard(app: Session, state: SlackImportController, job: Row) {
    val status = job.text("status")
    var confirmCancel by remember { mutableStateOf(false) }
    val exported = isSlackExport(job)
    SettingsSection(slackImportTitle(app, job), app.t("slackImport.status.$status")) {
        when (status) {
            "analyzing",
            "uploading" ->
                Text(
                    app.t("slackImport.status.analyzing"),
                    Modifier.then(webControlModifier("status")),
                    muted = true,
                )
            "ready" ->
                (job["analysis"] as? JsonObject)?.let { analysis ->
                    key(state.started) {
                        val channels = analysis.array("channels").rows()
                        SlackImportMappingForm(
                            app,
                            channels,
                            app.t(
                                "slackImport.summary",
                                "channels" to channels.size.toString(),
                                "users" to analysis.text("user_count"),
                            ),
                            state.pending,
                        ) {
                            state.start(job.text("id"), it)
                        }
                    }
                }
            "importing",
            "fetching_files",
            "failed" -> SlackImportRunStatus(app, job)
            "completed" -> {
                SlackImportRunStatus(app, job)
                if (slackImportRetryableFiles(job) > 0) SlackImportFileSummary(app, state, job)
            }
        }
        if (exported && status in setOf("uploading", "ready", "failed"))
            Button(app.t("slackImport.cancel"), enabled = !state.pending) { confirmCancel = true }
        if (!exported && status == "failed")
            Button(app.t("slackImport.dismiss"), enabled = !state.pending) {
                state.cancel(job.text("id"))
            }
    }
    if (confirmCancel)
        ConfirmationPanel(
            app.t("slackImport.cancelTitle"),
            app.t("slackImport.cancelDescription"),
            app.t("slackImport.cancel"),
            state.pending,
            null,
            { confirmCancel = false },
            {
                state.cancel(job.text("id"))
                confirmCancel = false
            },
        )
}

/** 未取得・取得失敗の添付の件数と、取得し直す操作。履歴の取り込みからも使う。 */
@Composable
private fun SlackImportFileSummary(app: Session, state: SlackImportController, job: Row) {
    val pending = job.text("pending_files").toIntOrNull() ?: 0
    val failed = job.text("failed_files").toIntOrNull() ?: 0
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (pending > 0)
            Text(app.t("slackImport.pendingFiles", "count" to pending.toString()), small = true, muted = true)
        if (failed > 0)
            Text(
                app.t("slackImport.failedFiles", "count" to failed.toString()),
                small = true,
                color = MaterialTheme.colorScheme.error,
            )
        Button(
            app.t("slackImport.fetchFiles"),
            variant = "outline",
            enabled = !state.pending,
            accessibleLabel = app.t("slackImport.fetchFilesFor", "name" to job.text("file_name")),
        ) {
            state.fetchFiles(job.text("id"))
        }
    }
}

/** 取り込みのジョブの進み具合・結果・失敗の理由 (React の `SlackImportRunStatus`) */
@Composable
internal fun SlackImportRunStatus(app: Session, job: Row) {
    when (job.text("status")) {
        "importing" -> {
            val done = job.text("processed_messages").toLongOrNull() ?: 0L
            val total = job.text("total_messages").toLongOrNull() ?: 0L
            SlackImportProgress(
                app.t(
                    "slackImport.progressMessages",
                    "done" to done.toString(),
                    "total" to total.toString(),
                ),
                if (total > 0) done.toFloat() / total else 0f,
            )
        }
        "fetching_files" -> {
            val done = job.text("processed_files").toLongOrNull() ?: 0L
            val total = job.text("total_files").toLongOrNull() ?: 0L
            SlackImportProgress(
                app.t(
                    "slackImport.progressFiles",
                    "done" to done.toString(),
                    "total" to total.toString(),
                ),
                if (total > 0) done.toFloat() / total else 0f,
            )
        }
        "completed" ->
            Text(
                app.t("slackImport.completed", "count" to job.text("processed_messages")),
                small = true,
            )
        "failed" ->
            job.text("error")
                .takeIf(String::isNotEmpty)
                ?.let {
                    Text(
                        it,
                        Modifier.then(webControlModifier("alert")),
                        small = true,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
    }
}

/**
 * チャンネルごとの取り込み方を選ぶフォーム (React の `MappingForm`)。管理者はすべての既存チャンネルを
 * 取り込み先に選べる。`targetsFor` を渡すと、チャンネルごとに選べる既存チャンネルを絞る (各ユーザーによる取り込み)。
 */
@Composable
internal fun SlackImportMappingForm(
    app: Session,
    channels: List<Row>,
    summary: String,
    pending: Boolean,
    targetsFor: ((Row) -> List<Row>)? = null,
    onSubmit: (List<Row>) -> Unit,
) {
    val zooChannels by app.repo.channels.rows.collectAsState()
    val targets =
        zooChannels
            .filter { !it.flag("is_dm") && it.text("deleted_at").isEmpty() }
            .sortedBy { it.text("channel_name") }
    // 読み直しで内容が変わっても、同じチャンネルの並びなら選んだ内容を残す
    var mappings by
        remember(channels.map { it.text("slack_channel_id") }) {
            mutableStateOf(slackImportDefaultMappings(channels))
        }
    fun update(index: Int, mapping: Row) {
        mappings = mappings.mapIndexed { i, item -> if (i == index) mapping else item }
    }
    val selected = mappings.count { it.text("action") != "skip" }
    val actionLabels =
        linkedMapOf(
            "create" to app.t("slackImport.actionCreate"),
            "existing" to app.t("slackImport.actionExisting"),
            "skip" to app.t("slackImport.actionSkip"),
        )
    Column(
        Modifier.fillMaxWidth().then(webControlModifier("form")),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(summary, muted = true)
        Column(
            Modifier.fillMaxWidth()
                .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
                .then(webControlModifier("list"))
        ) {
            channels.forEachIndexed { index, channel ->
                if (index > 0) HorizontalDivider()
                val mapping = mappings[index]
                val name = channel.text("name")
                val existing = channel["existing_channel"] as? JsonObject
                val deleted = channel.text("deleted_channel").isNotEmpty()
                // 非公開チャンネルの履歴は、メンバー以外も読める公開チャンネルには入れない
                val choices =
                    if (targetsFor != null) targetsFor(channel)
                    else if (channel.flag("is_private"))
                        targets.filter {
                            it.flag("is_private") || it.text("id") == existing?.text("id")
                        }
                    else targets
                // 以前取り込んだチャンネルは同じ取り込み先にしか入れられない (メッセージ ID が変わらないため)
                val actions =
                    actionLabels.keys.filter {
                        when (it) {
                            "create" -> existing == null && !deleted
                            "existing" -> choices.isNotEmpty() && !deleted
                            else -> true
                        }
                    }
                // React と同じく、広ければ名前を左・操作を右に並べ、狭ければ縦に積む
                BoxWithConstraints(
                    Modifier.fillMaxWidth().then(webControlModifier("listitem")).padding(12.dp)
                ) {
                    val wide = maxWidth >= 520.dp
                    @Composable
                    fun Info(modifier: Modifier) {
                        Column(modifier) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                if (channel.flag("is_private"))
                                    Icon(
                                        "lock",
                                        app.t("slackImport.private"),
                                        Modifier.size(14.dp),
                                    )
                                Text(
                                    "#$name",
                                    weight = androidx.compose.ui.text.font.FontWeight.Medium,
                                )
                            }
                            Text(
                                app.t(
                                    "slackImport.channelStats",
                                    "messages" to channel.text("message_count"),
                                    "members" to channel.text("members"),
                                ),
                                small = true,
                                muted = true,
                            )
                            if (existing != null)
                                Text(
                                    app.t("slackImport.importedTo", "name" to existing.text("name")),
                                    small = true,
                                    muted = true,
                                )
                            if (deleted)
                                Text(
                                    app.t(
                                        "slackImport.deletedTarget",
                                        "name" to channel.text("deleted_channel"),
                                    ),
                                    small = true,
                                    color = MaterialTheme.colorScheme.error,
                                )
                        }
                    }
                    @Composable
                    fun Controls() {
                        SelectField(
                            app.t("slackImport.actionFor", "name" to name),
                            actionLabels.getValue(mapping.text("action")),
                            actions.map(actionLabels::getValue),
                            enabled = !pending && actions.size > 1,
                            allowEmpty = false,
                        ) { label ->
                            val action = actionLabels.entries.first { it.value == label }.key
                            update(index, slackImportMappingFor(channel, action, choices))
                        }
                        when (mapping.text("action")) {
                            "create" ->
                                Field(
                                    mapping.text("name"),
                                    app.t("slackImport.channelName", "name" to name),
                                    Modifier.widthIn(max = 192.dp),
                                    showLabel = false,
                                    enabled = !pending,
                                ) {
                                    update(index, JsonObject(mapping + ("name" to JsonPrimitive(it))))
                                }
                            "existing" -> {
                                val labels =
                                    choices.associate { it.text("id") to "#${it.text("channel_name")}" }
                                SelectField(
                                    app.t("slackImport.targetChannel", "name" to name),
                                    labels[mapping.text("channel_id")] ?: "",
                                    labels.values.toList(),
                                    enabled = !pending && existing == null,
                                    allowEmpty = false,
                                ) { label ->
                                    val target = labels.entries.first { it.value == label }.key
                                    update(
                                        index,
                                        JsonObject(mapping + ("channel_id" to JsonPrimitive(target))),
                                    )
                                }
                            }
                        }
                    }
                    if (wide)
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Info(Modifier.weight(1f))
                            Controls()
                        }
                    else
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Info(Modifier.fillMaxWidth())
                            CssFlowRow(
                                Modifier.fillMaxWidth(),
                                horizontalGap = 8.dp,
                                verticalGap = 8.dp,
                            ) {
                                Controls()
                            }
                        }
                }
            }
        }
        Button(
            app.t("slackImport.start", "count" to selected.toString()),
            primary = true,
            enabled = !pending && selected > 0,
        ) {
            onSubmit(mappings)
        }
    }
}

@Composable
internal fun SlackImportProgress(label: String, value: Float) {
    // React の Math.round と同じく四捨五入する
    val percent = (value.coerceIn(0f, 1f) * 100).roundToInt()
    // 中身のない Box は Web のアクセシビリティ木に出ないので、ラベルを含む外側を progressbar にする
    Column(
        Modifier.fillMaxWidth()
            .then(
                webControlModifier(
                    "progressbar",
                    attributes =
                        mapOf(
                            "aria-label" to label,
                            "aria-valuemin" to "0",
                            "aria-valuemax" to "100",
                            "aria-valuenow" to percent.toString(),
                        ),
                )
            ),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(label, small = true, muted = true)
        Box(
            Modifier.fillMaxWidth()
                .height(8.dp)
                .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(50))
        ) {
            Box(
                Modifier.fillMaxHeight()
                    .fillMaxWidth(percent / 100f)
                    .background(MaterialTheme.colorScheme.primary, RoundedCornerShape(50))
            )
        }
    }
}
