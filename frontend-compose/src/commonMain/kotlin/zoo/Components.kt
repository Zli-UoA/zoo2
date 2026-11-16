package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

/**
 * メッセージ部品（Discord Components V2 形式。docs/apps.md「メッセージ部品」、Web の `ComponentTree.tsx`）。
 *
 * サーバーが検証した文書を読む。形が崩れていれば null を返し、呼び出し側は本文を表示する。
 * 知らない type は文字だけを表示する。
 */
const val IS_COMPONENTS_V2 = 1 shl 15

object CT {
    const val ACTION_ROW = 1
    const val BUTTON = 2
    const val STRING_SELECT = 3
    const val TEXT_INPUT = 4
    const val USER_SELECT = 5
    const val ROLE_SELECT = 6
    const val MENTIONABLE_SELECT = 7
    const val CHANNEL_SELECT = 8
    const val SECTION = 9
    const val TEXT_DISPLAY = 10
    const val THUMBNAIL = 11
    const val MEDIA_GALLERY = 12
    const val FILE = 13
    const val SEPARATOR = 14
    const val CONTAINER = 17
    const val LABEL = 18
    const val FILE_UPLOAD = 19
    const val RADIO_GROUP = 21
    const val CHECKBOX_GROUP = 22
    const val CHECKBOX = 23
    const val UNSUPPORTED = 1000
    const val EXTERNAL_SELECT = 1001
    const val OVERFLOW = 1002
    const val DATE_PICKER = 1003
    const val TIME_PICKER = 1004
    const val DATETIME_PICKER = 1005
    const val VIDEO = 1008
    const val TABLE = 1009
    const val ALERT = 1010
    const val CARD = 1011
    const val CAROUSEL = 1012
    const val CHART = 1013
    const val GRID = 1014
    const val PLAN = 1015
    const val TASK_CARD = 1016

    val NAMES =
        mapOf(
            "action_row" to 1,
            "button" to 2,
            "string_select" to 3,
            "text_input" to 4,
            "user_select" to 5,
            "role_select" to 6,
            "mentionable_select" to 7,
            "channel_select" to 8,
            "section" to 9,
            "text_display" to 10,
            "thumbnail" to 11,
            "media_gallery" to 12,
            "file" to 13,
            "separator" to 14,
            "container" to 17,
            "label" to 18,
            "file_upload" to 19,
            "radio_group" to 21,
            "checkbox_group" to 22,
            "checkbox" to 23,
        )
}

/** 部品の文書（`flags` と部品の配列）。部品は JSON のまま持ち、`type` だけを数値にそろえる。 */
data class ComponentDocument(val flags: Int, val components: List<JsonObject>) {
    val hidesContent: Boolean
        get() = flags and IS_COMPONENTS_V2 != 0
}

fun componentType(c: JsonObject): Int? {
    val raw = c["type"] as? JsonPrimitive ?: return null
    return raw.intOrNull ?: CT.NAMES[raw.content]
}

/** 保存した JSON（文字列か値）を読む。読めなければ null。 */
fun parseComponents(value: JsonElement?): ComponentDocument? {
    val element =
        (if (value is JsonPrimitive && value.isString)
            try {
                json.parseToJsonElement(value.content)
            } catch (_: Exception) {
                JsonNull
            }
        else value) as? JsonObject ?: return null
    val list = element["components"] as? JsonArray ?: return null
    if (list.size > 500) return null
    val components = list.mapNotNull { (it as? JsonObject)?.takeIf { c -> componentType(c) != null } }
    if (components.isEmpty()) return null
    val flags = (element["flags"] as? JsonPrimitive)?.intOrNull ?: 0
    return ComponentDocument(flags, components)
}

/** https の URL だけを通す。 */
fun safeHttps(url: String): String? =
    url.takeIf { it.startsWith("https://") && publicUrl(it) != null && '@' !in it.substringBefore('/', it.removePrefix("https://")) }

private fun JsonObject.cObj(key: String): JsonObject? = this[key] as? JsonObject

private fun JsonObject.cList(key: String): List<JsonObject> =
    (this[key] as? JsonArray)?.mapNotNull { it as? JsonObject }.orEmpty()

private fun JsonObject.cStr(key: String): String =
    (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content.orEmpty()

private fun JsonObject.cFlag(key: String): Boolean = (this[key] as? JsonPrimitive)?.booleanOrNull == true

private fun JsonObject.cInt(key: String): Int? = (this[key] as? JsonPrimitive)?.intOrNull

/** 選択を配列で送る部品か（select・checkbox group・file upload）。 */
fun multiValueComponent(type: Int?): Boolean =
    type in
        setOf(
            CT.STRING_SELECT,
            CT.USER_SELECT,
            CT.ROLE_SELECT,
            CT.MENTIONABLE_SELECT,
            CT.CHANNEL_SELECT,
            CT.EXTERNAL_SELECT,
            CT.CHECKBOX_GROUP,
            CT.FILE_UPLOAD,
        )

/** 部品の木の、最初の値（既定の選択・初期値）。モーダルの送信でまとめて送る。 */
fun initialComponentValues(doc: ComponentDocument): Map<String, JsonElement> {
    val out = mutableMapOf<String, JsonElement>()
    fun walk(c: JsonObject?) {
        if (c == null) return
        val type = componentType(c)
        val id = c.cStr("custom_id")
        if (id.isNotEmpty() && type != CT.BUTTON && type != CT.OVERFLOW) {
            out[id] =
                when (type) {
                    CT.TEXT_INPUT -> JsonPrimitive(c.cStr("value"))
                    CT.CHECKBOX -> JsonPrimitive(c.cFlag("default"))
                    CT.DATE_PICKER,
                    CT.TIME_PICKER,
                    CT.DATETIME_PICKER -> JsonPrimitive(c.cStr("initial_value"))
                    else -> {
                        val chosen =
                            c.cList("options").filter { it.cFlag("default") }.map { it.cStr("value") } +
                                c.cList("initial_options").map { it.cStr("value") } +
                                c.cList("default_values").map { it.cStr("id") }
                        if (multiValueComponent(type)) JsonArray(chosen.map(::JsonPrimitive))
                        else JsonPrimitive(chosen.firstOrNull().orEmpty())
                    }
                }
        }
        c.cList("components").forEach(::walk)
        walk(c.cObj("accessory"))
        walk(c.cObj("component"))
    }
    doc.components.forEach(::walk)
    return out
}

/** 操作の送り先と、入力の値。 */
private class ComponentScope(
    val app: Session,
    val appId: String?,
    val surface: String?,
    val message: String?,
    val channel: String?,
    val submit: Boolean,
    val attachments: List<Row>,
    val media: String,
    val values: Map<String, JsonElement>,
    val set: (String, JsonElement) -> Unit,
    val invoke: (kind: String, action: String?, value: JsonElement?) -> Unit,
    val busy: Boolean,
)

@Composable
fun ComponentTree(
    app: Session,
    doc: ComponentDocument,
    appId: String?,
    surface: String? = null,
    message: String? = null,
    channel: String? = null,
    submit: Boolean = false,
    attachments: List<Row> = emptyList(),
    done: suspend () -> Unit = {},
) {
    var values by remember(doc, appId, surface, message) { mutableStateOf(initialComponentValues(doc)) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    val sources by app.previews.endpoints.collectAsState()
    fun invoke(kind: String, action: String?, value: JsonElement?) {
        if (busy || appId == null) return
        app.scope.launch {
            busy = true
            error = ""
            try {
                app.api.request(
                    "/api/apps/$appId/invoke",
                    "POST",
                    buildJsonObject {
                        put("kind", kind)
                        if (surface != null) put("surface_id", surface)
                        if (message != null) put("message_id", message)
                        if (channel != null) put("channel_id", channel)
                        if (action != null) put("action_id", action)
                        put(
                            "values",
                            JsonObject(
                                values + if (action != null && value != null) mapOf(action to value) else emptyMap()
                            ),
                        )
                    },
                )
                app.invalidateSurfaces()
                if (kind == "submit") done()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                error = app.t("apps.actionError")
            } finally {
                busy = false
            }
        }
    }
    val scope =
        ComponentScope(
            app,
            appId,
            surface,
            message,
            channel,
            submit,
            attachments,
            sources.second,
            values,
            { k, v -> values = values + (k to v) },
            ::invoke,
            busy,
        )
    Column(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        doc.components.forEach { ComponentNode(scope, it) }
        if (submit)
            Button(app.t("apps.submit"), !busy && app.env.online, primary = true) { invoke("submit", null, null) }
        if (error.isNotEmpty()) Text(error, color = MaterialTheme.colorScheme.error)
    }
}

@Composable
private fun Small(text: String) {
    Text(text, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurface.copy(alpha = .65f))
}

@Composable
private fun ComponentImage(scope: ComponentScope, url: String, label: String, modifier: Modifier) {
    val proxied = safeHttps(url)?.let { proxyImageUrl(scope.media, it) } ?: return
    RemoteImage(proxied, label, modifier.clip(RoundedCornerShape(6.dp)))
}

@Composable
private fun Spoiler(spoiler: Boolean, content: @Composable () -> Unit) {
    var shown by remember { mutableStateOf(!spoiler) }
    if (shown) content()
    else
        Box(
            Modifier.clip(RoundedCornerShape(6.dp))
                .background(MaterialTheme.colorScheme.onSurface.copy(alpha = .12f))
                .clickable { shown = true }
                .padding(12.dp)
        ) {
            Small(LocalApp.current.t("apps.components.spoiler"))
        }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ComponentNode(scope: ComponentScope, c: JsonObject) {
    val app = scope.app
    when (componentType(c)) {
        CT.TEXT_DISPLAY -> if (c.cStr("content").isNotEmpty()) RichText(c.cStr("content"))
        CT.CONTAINER -> {
            val color = c.cInt("accent_color")?.let { Color(0xFF000000 or it.toLong()) } ?: MaterialTheme.colorScheme.primary
            var open by remember { mutableStateOf(!c.cFlag("collapsed")) }
            Row(
                Modifier.widthIn(max = 560.dp)
                    .height(IntrinsicSize.Min)
                    .clip(RoundedCornerShape(8.dp))
                    .border(1.dp, MaterialTheme.colorScheme.outline.copy(alpha = .3f), RoundedCornerShape(8.dp))
            ) {
                Box(Modifier.width(4.dp).fillMaxHeight().background(color))
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (c.cStr("title").isNotEmpty())
                        Row(
                            Modifier.then(if (c.cFlag("collapsible")) Modifier.clickable { open = !open } else Modifier),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            if (c.cFlag("collapsible")) Text(if (open) "▾ " else "▸ ")
                            RichText(c.cStr("title"))
                        }
                    if (c.cStr("subtitle").isNotEmpty()) Small(c.cStr("subtitle"))
                    if (open) Spoiler(c.cFlag("spoiler")) {
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            c.cList("components").forEach { ComponentNode(scope, it) }
                        }
                    }
                }
            }
        }
        CT.SECTION ->
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    c.cList("components").forEach { ComponentNode(scope, it) }
                }
                c.cObj("accessory")?.let { a ->
                    if (componentType(a) == CT.THUMBNAIL)
                        ComponentImage(scope, a.cObj("media")?.cStr("url").orEmpty(), a.cStr("description"), Modifier.size(64.dp))
                    else Interactive(scope, a)
                }
            }
        CT.THUMBNAIL -> ComponentImage(scope, c.cObj("media")?.cStr("url").orEmpty(), c.cStr("description"), Modifier.size(64.dp))
        CT.MEDIA_GALLERY ->
            FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                c.cList("items").take(10).forEach { item ->
                    Spoiler(item.cFlag("spoiler")) {
                        ComponentImage(scope, item.cObj("media")?.cStr("url").orEmpty(), item.cStr("description"), Modifier.width(160.dp).height(100.dp))
                    }
                }
            }
        CT.FILE -> {
            val url = c.cObj("file")?.cStr("url").orEmpty()
            val name = c.cStr("name").ifEmpty { url.substringAfterLast('/') }
            if (url.startsWith("attachment://")) {
                scope.attachments.firstOrNull { it.text("name") == url.removePrefix("attachment://") }?.let { file ->
                    Button(file.text("name"), app.env.online, variant = "link") {
                        app.run {
                            val result = app.api.request("/api/app-files/${file.text("id")}/download").jsonObjectOrNull()
                            val link = result?.text("url").orEmpty()
                            if (link.startsWith("https://")) app.platform.action("open:$link")
                        }
                    }
                }
            } else safeHttps(url)?.let { Button("📄 $name", variant = "link") { app.platform.action("open:$it") } }
        }
        CT.SEPARATOR ->
            if (c["divider"]?.let { (it as? JsonPrimitive)?.booleanOrNull } != false)
                Box(Modifier.fillMaxWidth().padding(vertical = if (c.cInt("spacing") == 2) 8.dp else 2.dp).height(1.dp).background(MaterialTheme.colorScheme.outline.copy(alpha = .3f)))
            else Box(Modifier.height(if (c.cInt("spacing") == 2) 16.dp else 4.dp))
        CT.ACTION_ROW ->
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                c.cList("components").forEach { Interactive(scope, it) }
            }
        CT.LABEL ->
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(c.cStr("label"), fontWeight = FontWeight.Medium)
                if (c.cStr("description").isNotEmpty()) Small(c.cStr("description"))
                c.cObj("component")?.let { Interactive(scope, it, labelled = true) }
            }
        CT.VIDEO -> {
            val href = safeHttps(c.cStr("title_url")) ?: safeHttps(c.cStr("video_url"))
            Column(Modifier.widthIn(max = 320.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (c.cStr("thumbnail_url").isNotEmpty())
                    ComponentImage(scope, c.cStr("thumbnail_url"), c.cStr("alt_text"), Modifier.fillMaxWidth().height(160.dp))
                Button("▶ ${c.cStr("title")}", href != null, variant = "link") { href?.let { app.platform.action("open:$it") } }
                if (c.cStr("description").isNotEmpty()) Small(c.cStr("description"))
            }
        }
        CT.TABLE -> ComponentTable(scope, c)
        CT.ALERT ->
            Box(
                Modifier.fillMaxWidth()
                    .clip(RoundedCornerShape(6.dp))
                    .background(
                        when (c.cStr("level")) {
                            "error" -> MaterialTheme.colorScheme.error.copy(alpha = .12f)
                            "warning" -> Color(0x33E0A000)
                            "success" -> Color(0x3322AA66)
                            else -> MaterialTheme.colorScheme.onSurface.copy(alpha = .06f)
                        }
                    )
                    .padding(horizontal = 12.dp, vertical = 8.dp)
            ) {
                RichText(c.cStr("content"))
            }
        CT.CARD -> ComponentCard(scope, c)
        CT.CAROUSEL ->
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                c.cList("cards").forEach { ComponentCard(scope, it) }
            }
        CT.CHART -> {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(c.cStr("title"), fontWeight = FontWeight.Medium)
                val segments = c.cList("segments")
                val series = c.cList("series")
                val values = segments.map { it.cNum("value") } + series.flatMap { s -> s.cList("data").map { it.cNum("value") } }
                val max = (values.maxOrNull() ?: 1.0).coerceAtLeast(1e-9)
                segments.forEach { BarLine(it.cStr("label"), it.cNum("value"), max) }
                series.forEach { s ->
                    Small(s.cStr("name"))
                    s.cList("data").forEach { BarLine(it.cStr("label"), it.cNum("value"), max) }
                }
            }
        }
        CT.GRID ->
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (c.cStr("label").isNotEmpty()) Text(c.cStr("label"), fontWeight = FontWeight.Medium)
                (c["rows"] as? JsonArray)?.forEach { row ->
                    // 狭い画面では縦に積む（Web の @container と同じ考え方）
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        (row as? JsonArray)?.mapNotNull { it as? JsonObject }?.forEach { item ->
                            item.cObj("component")?.let { ComponentNode(scope, it) }
                        }
                    }
                }
            }
        CT.PLAN ->
            Column(
                Modifier.border(1.dp, MaterialTheme.colorScheme.outline.copy(alpha = .3f), RoundedCornerShape(8.dp)).padding(8.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(c.cStr("title"), fontWeight = FontWeight.Medium)
                c.cList("tasks").forEach { TaskLine(it) }
            }
        CT.TASK_CARD -> TaskLine(c)
        CT.UNSUPPORTED -> if (c.cStr("text").isNotEmpty()) Small(c.cStr("text"))
        null -> {}
        else -> Interactive(scope, c)
    }
}

private fun JsonObject.cNum(key: String): Double = (this[key] as? JsonPrimitive)?.doubleOrNull ?: 0.0

@Composable
private fun BarLine(label: String, value: Double, max: Double) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(label, fontSize = 12.sp, modifier = Modifier.width(72.dp))
        Box(
            Modifier.height(8.dp)
                .width((160 * (value / max).coerceIn(0.0, 1.0)).dp)
                .background(MaterialTheme.colorScheme.primary, RoundedCornerShape(2.dp))
        )
        Text(value.toString().removeSuffix(".0"), fontSize = 12.sp)
    }
}

@Composable
private fun TaskLine(task: JsonObject) {
    val mark =
        when (task.cStr("status")) {
            "complete" -> "✓"
            "error" -> "✕"
            "in_progress" -> "…"
            else -> "○"
        }
    Column {
        Text("$mark ${task.cStr("title")}")
        if (task.cStr("details").isNotEmpty()) Small(task.cStr("details"))
        if (task.cStr("output").isNotEmpty()) Small(task.cStr("output"))
    }
}

@Composable
private fun ComponentCard(scope: ComponentScope, c: JsonObject) {
    Column(
        Modifier.width(240.dp)
            .clip(RoundedCornerShape(8.dp))
            .border(1.dp, MaterialTheme.colorScheme.outline.copy(alpha = .3f), RoundedCornerShape(8.dp))
    ) {
        c.cObj("hero_image")?.let { ComponentImage(scope, it.cStr("url"), "", Modifier.fillMaxWidth().height(120.dp)) }
        Column(Modifier.padding(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            if (c.cStr("title").isNotEmpty()) RichText(c.cStr("title"))
            if (c.cStr("subtitle").isNotEmpty()) Small(c.cStr("subtitle"))
            if (c.cStr("body").isNotEmpty()) RichText(c.cStr("body"))
            if (c.cStr("subtext").isNotEmpty()) Small(c.cStr("subtext"))
            c.cList("actions").forEach { Interactive(scope, it) }
        }
    }
}

@Composable
private fun ComponentTable(scope: ComponentScope, c: JsonObject) {
    val rows = (c["rows"] as? JsonArray)?.mapNotNull { row -> (row as? JsonArray)?.mapNotNull { it as? JsonObject } }.orEmpty()
    val header = c.cFlag("header")
    val pageSize = c.cInt("page_size") ?: rows.size.coerceAtLeast(1)
    var page by remember { mutableStateOf(0) }
    val body = if (header) rows.drop(1) else rows
    val pages = ((body.size + pageSize - 1) / pageSize).coerceAtLeast(1)
    Column(Modifier.horizontalScroll(rememberScrollState())) {
        if (c.cStr("caption").isNotEmpty()) Small(c.cStr("caption"))
        (if (header) listOf(rows.first()) else emptyList()).plus(body.drop(page * pageSize).take(pageSize)).forEachIndexed { i, row ->
            Row(Modifier.padding(vertical = 2.dp)) {
                row.forEach { cell ->
                    Box(Modifier.width(120.dp).padding(horizontal = 4.dp)) {
                        val button = cell.cObj("button")
                        if (button != null) Interactive(scope, button)
                        else if (header && i == 0) Text(cell.cStr("text"), fontWeight = FontWeight.Medium)
                        else RichText(cell.cStr("text"))
                    }
                }
            }
        }
        if (pages > 1)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Button("‹", page > 0, size = "sm") { page-- }
                Small(scope.app.t("apps.components.page", "page" to "${page + 1}", "pages" to "$pages"))
                Button("›", page + 1 < pages, size = "sm") { page++ }
            }
    }
}

/** ボタン・select・入力欄・ピッカー。`labelled` は Label の中（送信でまとめて送る）。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Interactive(scope: ComponentScope, c: JsonObject, labelled: Boolean = false) {
    val app = scope.app
    val id = c.cStr("custom_id")
    val type = componentType(c)
    val enabled = !c.cFlag("disabled") && !scope.busy && scope.appId != null && app.env.online
    val deferred = labelled || scope.submit
    var confirming by remember { mutableStateOf<(() -> Unit)?>(null) }
    val confirm = c.cObj("confirm")
    fun run(action: () -> Unit) {
        if (confirm != null) confirming = action else action()
    }
    fun choose(value: JsonElement) {
        scope.set(id, value)
        if (!deferred) run { scope.invoke("action", id, value) }
    }
    confirming?.let { action ->
        Confirm(
            confirm?.cStr("title").orEmpty(),
            confirm?.cStr("text").orEmpty(),
            confirm?.cStr("confirm").orEmpty(),
            busy = false,
            cancel = { confirming = null },
        ) {
            confirming = null
            action()
        }
    }
    val options = c.cList("options")
    when (type) {
        CT.BUTTON -> {
            val label = listOf(c.cObj("emoji")?.cStr("name").orEmpty(), c.cStr("label")).filter { it.isNotEmpty() }.joinToString(" ")
            if (c.cInt("style") == 5) {
                val href = safeHttps(c.cStr("url"))
                Button(label, href != null, variant = "outline", size = "sm") { href?.let { app.platform.action("open:$it") } }
            } else
                Button(
                    label,
                    enabled,
                    variant =
                        when (c.cInt("style")) {
                            1 -> "default"
                            3 -> "secondary"
                            4 -> "destructive"
                            else -> "outline"
                        },
                    size = "sm",
                ) {
                    run { scope.invoke("action", id, JsonPrimitive(c.cStr("value"))) }
                }
        }
        CT.STRING_SELECT,
        CT.OVERFLOW -> {
            val many = type == CT.STRING_SELECT && (c.cInt("max_values") ?: 1) > 1
            if (many) CheckList(scope, id, options, enabled || deferred) { choose(it) }
            else {
                val current = (scope.values[id] as? JsonArray)?.firstOrNull()?.let { (it as? JsonPrimitive)?.content }
                    ?: (scope.values[id] as? JsonPrimitive)?.content.orEmpty()
                SelectField(
                    c.cStr("placeholder").ifEmpty { if (type == CT.OVERFLOW) "…" else app.t("apps.components.choose") },
                    options.firstOrNull { it.cStr("value") == current }?.cStr("label").orEmpty(),
                    options.map { it.cStr("label") },
                    enabled || deferred,
                ) { label ->
                    val value = options.firstOrNull { it.cStr("label") == label }?.cStr("value") ?: return@SelectField
                    choose(if (type == CT.OVERFLOW) JsonPrimitive(value) else JsonArray(listOf(JsonPrimitive(value))))
                }
            }
        }
        CT.RADIO_GROUP ->
            Column {
                options.forEach { o ->
                    Row(
                        Modifier.clickable(enabled = enabled || deferred) { choose(JsonPrimitive(o.cStr("value"))) },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton((scope.values[id] as? JsonPrimitive)?.content == o.cStr("value"), null)
                        OptionText(o)
                    }
                }
            }
        CT.CHECKBOX_GROUP -> CheckList(scope, id, options, enabled || deferred) { choose(it) }
        CT.CHECKBOX ->
            Checkbox((scope.values[id] as? JsonPrimitive)?.booleanOrNull == true, { choose(JsonPrimitive(it)) }, enabled = enabled || deferred)
        CT.TEXT_INPUT -> {
            val current = (scope.values[id] as? JsonPrimitive)?.content.orEmpty()
            Field(
                current,
                c.cStr("placeholder").ifEmpty { c.cStr("label") },
                singleLine = c.cInt("style") != 2 && c.cStr("format") != "rich",
                showLabel = false,
            ) {
                scope.set(id, JsonPrimitive(it.take(c.cInt("max_length") ?: 4000)))
            }
        }
        CT.DATE_PICKER,
        CT.TIME_PICKER,
        CT.DATETIME_PICKER -> {
            // 端末の日付ピッカーは使わず、書式を示した入力欄にする（date: YYYY-MM-DD、time: HH:mm、datetime: UNIX 秒）
            val hint =
                when (type) {
                    CT.DATE_PICKER -> "YYYY-MM-DD"
                    CT.TIME_PICKER -> "HH:mm"
                    else -> "UNIX"
                }
            var text by remember(id) { mutableStateOf((scope.values[id] as? JsonPrimitive)?.content.orEmpty()) }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(Modifier.width(160.dp)) {
                    Field(text, c.cStr("placeholder").ifEmpty { hint }, showLabel = false) {
                        text = it.take(32)
                        scope.set(id, JsonPrimitive(text))
                    }
                }
                if (!deferred) Button(app.t("apps.submit"), enabled && text.isNotBlank(), size = "sm") { choose(JsonPrimitive(text)) }
            }
        }
        CT.USER_SELECT,
        CT.CHANNEL_SELECT,
        CT.MENTIONABLE_SELECT,
        CT.ROLE_SELECT -> EntitySelect(scope, c, enabled || deferred) { choose(it) }
        CT.EXTERNAL_SELECT -> ExternalSelect(scope, c, enabled || deferred) { choose(it) }
        CT.FILE_UPLOAD -> Small(app.t("apps.components.webOnly"))
    }
}

@Composable
private fun OptionText(o: JsonObject) {
    Column {
        Text(listOf(o.cObj("emoji")?.cStr("name").orEmpty(), o.cStr("label")).filter { it.isNotEmpty() }.joinToString(" "))
        if (o.cStr("description").isNotEmpty()) Small(o.cStr("description"))
    }
}

@Composable
private fun CheckList(scope: ComponentScope, id: String, options: List<JsonObject>, enabled: Boolean, change: (JsonElement) -> Unit) {
    val chosen = (scope.values[id] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.content }.orEmpty()
    Column {
        options.forEach { o ->
            val value = o.cStr("value")
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    value in chosen,
                    { on -> change(JsonArray((if (on) chosen + value else chosen - value).map(::JsonPrimitive))) },
                    enabled = enabled,
                )
                OptionText(o)
            }
        }
    }
}

/** user / channel select。同期済みのユーザーとチャンネルから選ぶ（別に取り直さない）。 */
@Composable
private fun EntitySelect(scope: ComponentScope, c: JsonObject, enabled: Boolean, change: (JsonElement) -> Unit) {
    val app = scope.app
    val type = componentType(c)
    val channels = type == CT.CHANNEL_SELECT || type == CT.ROLE_SELECT
    val choices: List<Pair<String, String>> =
        if (channels) app.repo.channels.rows.value.filter { it.text("deleted_at").isEmpty() }.map { it.text("id") to "#${it.text("channel_name")}" }
        else app.repo.users.rows.value.map { it.text("id") to "@${it.text("user_name")}" }
    val current = (scope.values[c.cStr("custom_id")] as? JsonArray)?.firstOrNull()?.let { (it as? JsonPrimitive)?.content }
    SelectField(
        c.cStr("placeholder").ifEmpty { app.t("apps.components.choose") },
        choices.firstOrNull { it.first == current }?.second.orEmpty(),
        choices.take(500).map { it.second },
        enabled,
    ) { label ->
        choices.firstOrNull { it.second == label }?.let { change(JsonArray(listOf(JsonPrimitive(it.first)))) }
    }
}

/**
 * 入力に合わせてアプリに選択肢を問い合わせる select。入力が止まって 400ms 後に 1 回だけ問い合わせる
 * （入力が変わらなければ問い合わせない。応答はサーバーが最大 3 秒待つ）。
 */
@Composable
private fun ExternalSelect(scope: ComponentScope, c: JsonObject, enabled: Boolean, change: (JsonElement) -> Unit) {
    val app = scope.app
    var query by remember { mutableStateOf("") }
    var options by remember { mutableStateOf(c.cList("initial_options")) }
    LaunchedEffect(query) {
        if (query.length < (c.cInt("min_query_length") ?: 3) || scope.appId == null) return@LaunchedEffect
        delay(400)
        try {
            val asked =
                app.api.request(
                    "/api/apps/${scope.appId}/invoke",
                    "POST",
                    buildJsonObject {
                        put("kind", "options")
                        scope.surface?.let { put("surface_id", it) }
                        scope.message?.let { put("message_id", it) }
                        scope.channel?.let { put("channel_id", it) }
                        put("action_id", c.cStr("custom_id"))
                        put("values", buildJsonObject { put("query", query) })
                    },
                ).jsonObjectOrNull()
            val request = asked?.text("request_id").orEmpty()
            if (request.isNotEmpty()) {
                val got = app.api.request("/api/apps/${scope.appId}/options/$request").jsonObjectOrNull()
                (got?.get("options") as? JsonArray)?.let { list -> options = list.mapNotNull { it as? JsonObject } }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {}
    }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Field(query, c.cStr("placeholder").ifEmpty { app.t("apps.components.search") }, showLabel = false) { query = it.take(100) }
        val current = (scope.values[c.cStr("custom_id")] as? JsonArray)?.firstOrNull()?.let { (it as? JsonPrimitive)?.content }
        SelectField(
            if (options.isEmpty()) app.t("apps.components.noOptions") else app.t("apps.components.choose"),
            options.firstOrNull { it.cStr("value") == current }?.cStr("label").orEmpty(),
            options.map { it.cStr("label") },
            enabled && options.isNotEmpty(),
        ) { label ->
            options.firstOrNull { it.cStr("label") == label }?.let { change(JsonArray(listOf(JsonPrimitive(it.cStr("value"))))) }
        }
    }
}

private fun JsonElement?.jsonObjectOrNull(): JsonObject? = this as? JsonObject
