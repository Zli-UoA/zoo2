package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.key.*
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.unit.*
import kotlinx.serialization.json.*
import zoo_compose.generated.resources.Res

fun codePoints(value: String): List<Int> {
    val out = mutableListOf<Int>()
    var i = 0
    while (i < value.length) {
        val first = value[i++]
        out +=
            if (first.isHighSurrogate() && i < value.length && value[i].isLowSurrogate())
                0x10000 + ((first.code - 0xd800) shl 10) + value[i++].code - 0xdc00
            else first.code
    }
    return out
}

fun kitchenComponent(value: String): String? {
    val points =
        if (Regex("^[0-9a-fA-F]+(-[0-9a-fA-F]+)*$").matches(value))
            value.split('-').map { it.toIntOrNull(16) ?: return null }
        else codePoints(value)
    if (points.isEmpty() || points.any { it !in 0..0x10ffff || it in 0xd800..0xdfff }) return null
    return points
        .filter { it != 0xfe0f }
        .joinToString("-") { it.toString(16) }
        .takeIf { it.isNotEmpty() }
}

fun reactionKey(value: String): String {
    if (!value.startsWith("kitchen:")) return value
    val parts = value.removePrefix("kitchen:").split('_').takeIf { it.size == 2 } ?: return value
    val components = parts.map { kitchenComponent(it) ?: return value }
    return "kitchen:${components.sorted().joinToString("_")}"
}

fun kitchenLabel(value: String): String {
    if (!value.startsWith("kitchen:")) return value
    val parts = value.removePrefix("kitchen:").split('_').takeIf { it.size == 2 } ?: return value
    return parts
        .map { kitchenComponent(it)?.let(::componentGlyph) ?: return value }
        .joinToString(" + ")
}

fun kitchenImage(catalog: Row, value: String): String? {
    val parts =
        reactionKey(value).removePrefix("kitchen:").split('_').takeIf { it.size == 2 }
            ?: return null
    val emojis = catalog.array("emojis").map { it.jsonArray }
    val indices =
        parts
            .map { code ->
                emojis.indexOfFirst { kitchenComponent(it[0].jsonPrimitive.content) == code }
            }
            .sorted()
    if (indices.any { it < 0 }) return null
    val packed =
        catalog.nested("pairs")[indices.joinToString("_")]?.jsonPrimitive?.intOrNull ?: return null
    val ordered = if (packed % 2 == 0) indices else indices.reversed()
    val (left, right) =
        ordered.map { index ->
            emojis[index][0].jsonPrimitive.content.split('-').joinToString("-") { "u$it" }
        }
    val date = catalog.array("dates")[packed / 2].jsonPrimitive.content
    return "https://www.gstatic.com/android/keyboard/emojikitchen/$date/$left/${left}_$right.png"
}

fun componentGlyph(code: String): String = buildString {
    code.split('-').forEach { part ->
        val point = part.toIntOrNull(16) ?: return@forEach
        if (point <= 0xffff) append(point.toChar())
        else {
            val value = point - 0x10000
            append((0xd800 + (value shr 10)).toChar())
            append((0xdc00 + (value and 1023)).toChar())
        }
    }
}

@Composable
fun ReactionEmoji(
    value: String,
    modifier: Modifier = Modifier,
    catalog: Row? = null,
    loadCatalog: Boolean = true,
    size: Dp = 20.dp,
) {
    val app = LocalApp.current
    var url by
        remember(value, catalog) {
            mutableStateOf<String?>(catalog?.let { kitchenImage(it, value) })
        }
    LaunchedEffect(value, catalog, loadCatalog) {
        if (Regex("^:[a-z0-9_-]{1,64}:$").matches(value)) {
            try {
                url =
                    app.api
                        .request("/api/emojis/${value.removeSurrounding(":")}")
                        .jsonObject
                        .text("url")
                        .takeIf { it.startsWith("https://") }
            } catch (_: Exception) {}
        } else if (value.startsWith("kitchen:") && catalog == null && loadCatalog) {
            try {
                url =
                    kitchenImage(
                        json
                            .parseToJsonElement(
                                Res.readBytes("files/emoji-kitchen.json").decodeToString()
                            )
                            .jsonObject,
                        value,
                    )
            } catch (_: Exception) {}
        }
    }
    if (url != null)
        RemoteImage(url!!, if (value.startsWith("kitchen:")) "" else value, modifier.size(size))
    else if (value.startsWith("kitchen:"))
        Box(modifier.size(size), contentAlignment = Alignment.Center) {
            Text(
                componentGlyph(value.removePrefix("kitchen:").substringBefore('_')),
                size = size.value.sp,
                lineHeight = size.value.sp,
            )
        }
    else if (value.startsWith(":")) Text(value, modifier)
    else
        Box(modifier.size(size), contentAlignment = Alignment.Center) {
            Text(value, size = size.value.sp, lineHeight = size.value.sp)
        }
}

@Composable
fun KitchenPicker(app: Session, select: (String) -> Unit, catalogUrl: String? = null) {
    var catalog by remember { mutableStateOf<Row?>(null) }
    var left by remember { mutableStateOf("1f600") }
    var right by remember { mutableStateOf("1f431") }
    var error by remember { mutableStateOf(false) }
    var attempt by remember { mutableStateOf(0) }
    LaunchedEffect(catalogUrl, attempt) {
        error = false
        try {
            catalog =
                if (catalogUrl != null) app.asset(catalogUrl).jsonObject
                else
                    json
                        .parseToJsonElement(
                            Res.readBytes("files/emoji-kitchen.json").decodeToString()
                        )
                        .jsonObject
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (_: Exception) {
            error = true
        }
    }
    val data = catalog
    val colors = androidx.compose.material3.MaterialTheme.colorScheme
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("reactions.firstEmoji" to left, "reactions.secondEmoji" to right)
                .forEachIndexed { index, (label, selected) ->
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(app.t(label), small = true)
                        if (data == null)
                            Box(
                                Modifier.fillMaxWidth()
                                    .height(32.dp)
                                    .background(
                                        colors.surfaceVariant,
                                        androidx.compose.foundation.shape.RoundedCornerShape(8.dp),
                                    )
                            )
                        else {
                            KitchenEmojiField(app, app.t(label), data, selected) { code ->
                                if (index == 0) left = code else right = code
                            }
                        }
                    }
                }
        }
        val value = reactionKey("kitchen:${left}_$right")
        val url = data?.let { kitchenImage(it, value) }
        var previewLoaded by remember(url) { mutableStateOf(false) }
        var previewFailed by remember(url) { mutableStateOf(false) }
        Column(
            Modifier.fillMaxWidth()
                .height(160.dp)
                .border(
                    1.dp,
                    colors.outline,
                    androidx.compose.foundation.shape.RoundedCornerShape(8.dp),
                ),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            if (data == null) {
                Text(
                    app.t(if (error) "reactions.kitchenLoadError" else "reactions.kitchenLoading"),
                    color = if (error) colors.error else colors.onBackground,
                )
                if (error) Button(app.t("reactions.retry"), variant = "outline") { attempt++ }
            } else if (url != null) {
                RemoteImage(
                    url,
                    app.t("reactions.previewAlt"),
                    Modifier.size(80.dp),
                    onLoaded = { previewLoaded = true },
                    onError = { previewFailed = true },
                )
                if (!previewLoaded)
                    Text(
                        app.t(
                            if (previewFailed) "reactions.previewError"
                            else "reactions.previewLoading"
                        ),
                        small = true,
                        muted = !previewFailed,
                        color = if (previewFailed) colors.error else colors.onSurfaceVariant,
                    )
            } else {
                ReactionEmoji(value, catalog = data, loadCatalog = false, size = 64.dp)
                Text(app.t("reactions.unsupported"), small = true, muted = true)
            }
        }
        Button(
            app.t("reactions.addCombination"),
            primary = true,
            enabled = url != null && previewLoaded,
            modifier = Modifier.fillMaxWidth(),
        ) {
            select(value)
        }
        Text(app.t("reactions.kitchenImageNotice"), small = true, muted = true)
    }
}

@Composable
private fun KitchenEmojiField(
    app: Session,
    label: String,
    catalog: Row,
    value: String,
    change: (String) -> Unit,
) {
    val entries = catalog.array("emojis").map { it.jsonArray }
    fun name(entry: JsonArray) =
        "${entry[1].jsonPrimitive.content} ${entry[2].jsonPrimitive.content.replace('_',' ')}"
    val selected = entries.find { it[0].jsonPrimitive.content == value }
    var query by remember(value) { mutableStateOf(selected?.let(::name).orEmpty()) }
    var open by remember { mutableStateOf(false) }
    var anchor by remember { mutableStateOf(Rect.Zero) }
    val filtered =
        entries.filter { query == selected?.let(::name) || name(it).contains(query, true) }
    fun choose(entry: JsonArray) {
        change(entry[0].jsonPrimitive.content)
        query = name(entry)
        open = false
    }
    Field(
        query,
        label,
        Modifier.fillMaxWidth()
            .onGloballyPositioned { anchor = it.boundsInRoot() }
            .onFocusChanged { if (it.isFocused) open = true }
            .onPreviewKeyEvent {
                if (
                    it.type == KeyEventType.KeyDown &&
                        it.key == Key.Enter &&
                        open &&
                        filtered.isNotEmpty()
                ) {
                    choose(filtered.first())
                    true
                } else if (it.type == KeyEventType.KeyDown && it.key == Key.Escape && open) {
                    open = false
                    true
                } else false
            },
        showLabel = false,
        role = "combobox",
        placeholder = app.t("reactions.searchEmoji"),
        leadingIcon = "search",
    ) {
        query = it
        open = true
    }
    if (open)
        OverlayRegistration(label, { open = false }, 224.dp, anchor) {
            LazyColumn(
                Modifier.heightIn(max = 192.dp)
                    .then(webControlModifier("listbox", attributes = mapOf("aria-label" to label)))
            ) {
                if (filtered.isEmpty())
                    item { Text(app.t("reactions.noSearchResults"), small = true, muted = true) }
                items(filtered) { entry ->
                    Box(
                        Modifier.fillMaxWidth()
                            .then(
                                webControlModifier(
                                    "option",
                                    selected = entry[0].jsonPrimitive.content == value,
                                )
                            )
                            .clickable { choose(entry) }
                            .padding(horizontal = 8.dp, vertical = 6.dp)
                    ) {
                        Text(name(entry))
                    }
                }
            }
        }
}
