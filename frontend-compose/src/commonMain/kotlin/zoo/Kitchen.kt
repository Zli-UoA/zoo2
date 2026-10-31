package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
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

@Composable
fun ReactionEmoji(value: String, modifier: Modifier = Modifier) {
    val app = LocalApp.current
    var url by remember(value) { mutableStateOf<String?>(null) }
    LaunchedEffect(value) {
        if (value.startsWith("kitchen:")) {
            try {
                val catalog =
                    json
                        .parseToJsonElement(
                            Res.readBytes("files/emoji-kitchen.json").decodeToString()
                        )
                        .jsonObject
                url = kitchenImage(catalog, value)
            } catch (_: Exception) {}
        }
    }
    if (url != null) RemoteImage(url!!, value.removePrefix("kitchen:"), modifier.size(28.dp))
    else Text(value, modifier)
}

@Composable
fun KitchenPicker(app: Session, select: (String) -> Unit) {
    var catalog by remember { mutableStateOf<Row?>(null) }
    var search by remember { mutableStateOf("") }
    var left by remember { mutableStateOf<String?>(null) }
    var right by remember { mutableStateOf<String?>(null) }
    var slot by remember { mutableStateOf(0) }
    LaunchedEffect(Unit) {
        try {
            catalog =
                json
                    .parseToJsonElement(Res.readBytes("files/emoji-kitchen.json").decodeToString())
                    .jsonObject
        } catch (_: Exception) {}
    }
    Field(search, app.t("channels.browseSearch")) { search = it }
    Row {
        Button("${app.t("reactions.firstEmoji")}: ${left.orEmpty()}") { slot = 0 }
        Button("${app.t("reactions.secondEmoji")}: ${right.orEmpty()}") { slot = 1 }
    }
    val data = catalog
    if (data == null) Text(app.t("reactions.kitchenLoading"))
    else {
        LazyColumn(Modifier.height(220.dp)) {
            items(
                data
                    .array("emojis")
                    .map { it.jsonArray }
                    .filter { it.toString().contains(search, true) }
                    .chunked(7)
            ) { group ->
                Row {
                    group.forEach { entry ->
                        Button(entry[1].jsonPrimitive.content) {
                            if (slot == 0) {
                                left = entry[0].jsonPrimitive.content
                                slot = 1
                            } else right = entry[0].jsonPrimitive.content
                        }
                    }
                }
            }
        }
        if (left != null && right != null) {
            val value = reactionKey("kitchen:${left}_$right")
            val url = kitchenImage(data, value)
            if (url != null) {
                RemoteImage(url, app.t("reactions.previewAlt"), Modifier.size(96.dp))
                Button(app.t("reactions.addCombination"), primary = true) { select(value) }
            } else Text(app.t("reactions.unsupported"), muted = true)
        }
    }
}
