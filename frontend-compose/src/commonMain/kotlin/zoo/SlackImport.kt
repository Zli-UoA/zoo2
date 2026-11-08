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

// 各ユーザーの Slack 取り込み (SlackLink.kt) が使う部品。管理者の取り込み画面は React 版の Web だけで提供する。

/** ワーカーが処理中で、結果を待つ必要がある状態 (React の `SLACK_IMPORT_RUNNING`) */
val slackImportRunning = setOf("uploading", "analyzing", "importing", "fetching_files")

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

/**
 * 以前に取り込んだチャンネルか。取り込み先が見えないチャンネル (参加していない非公開チャンネルなど) のときは、
 * サーバーは名前と ID を返さず `existing_hidden` だけを返す。どちらも同じ取り込み先にしか入れられない
 */
fun slackImportImported(channel: Row): Boolean =
    channel["existing_channel"] is JsonObject || channel.flag("existing_hidden")

/**
 * 「既存のチャンネル」で選べる取り込み先 (React の `targetChoices`)。非公開チャンネルの履歴は公開チャンネルに
 * 入れない。以前の取り込み先は、参加していない非公開チャンネルで同期されていなくても選べるよう先頭に足す。
 */
fun slackImportChoices(channel: Row, targets: List<Row>): List<Row> {
    val existing = channel["existing_channel"] as? JsonObject
    val choices =
        if (channel.flag("is_private"))
            targets.filter { it.flag("is_private") || it.text("id") == existing?.text("id") }
        else targets
    if (existing == null || choices.any { it.text("id") == existing.text("id") }) return choices
    val private = (existing["is_private"] as? JsonPrimitive)?.booleanOrNull ?: channel.flag("is_private")
    return listOf(
        obj(
            "id" to existing.text("id"),
            "channel_name" to existing.text("name"),
            "is_private" to private,
        )
    ) + choices
}

/** 失敗の理由。React の `describeImportError` と同じく、403 と `{error}` を見分ける */
data class SlackImportFailure(val status: Int?, val message: String?)

fun slackImportFailure(cause: Throwable): SlackImportFailure =
    (cause as? ApiException)?.let {
        SlackImportFailure(it.status, it.body?.text("error")?.takeIf(String::isNotEmpty))
    } ?: SlackImportFailure(null, null)

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
 * 各ユーザーが自分の Slack チャンネルの取り込み方を選ぶフォーム (React の `MappingForm`)。
 * 選べる既存チャンネルは、サーバーが返す取り込み先 (`targetsFor`) に絞る。
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
                val imported = slackImportImported(channel)
                val deleted = channel.text("deleted_channel").isNotEmpty()
                // セルフ取り込みでは、サーバーが返す取り込み先だけを選べる
                val choices = targetsFor?.invoke(channel) ?: slackImportChoices(channel, targets)
                // 以前取り込んだチャンネルは同じ取り込み先にしか入れられない (メッセージ ID が変わらないため)
                val actions =
                    actionLabels.keys.filter {
                        when (it) {
                            "create" -> !imported && !deleted
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
                            if (channel.flag("existing_hidden"))
                                Text(app.t("slackImport.importedToHidden"), small = true, muted = true)
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
                                    enabled = !pending && !imported,
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
