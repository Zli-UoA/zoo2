package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.serialization.json.*

@Composable
fun workspaceEmojis(app: Session): List<Row> {
    LaunchedEffect(app) { try { app.loadWorkspaceEmojis() } catch (e: CancellationException) { throw e } catch (_: Exception) {} }
    return app.workspaceEmojis
}

fun workspaceEmoji(value: Row): Row = buildJsonObject {
    put("name", ":${value.text("name")}:")
    put("url", value.text("url")); put("image_id", value.text("url")); put("local", true)
}

@Composable
fun CustomEmojiSettings(app: Session) {
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf("") }
    var category by remember { mutableStateOf("") }
    var aliases by remember { mutableStateOf("") }
    var previous by remember { mutableStateOf<Row?>(null) }
    var file by remember { mutableStateOf<Row?>(null) }
    var busy by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(true) }
    var failed by remember { mutableStateOf(false) }
    var remove by remember { mutableStateOf<Row?>(null) }
    fun reset() { name = ""; category = ""; aliases = ""; previous = null; file?.let { app.platform.action("fileRelease:${it.text("id")}") }; file = null }
    fun load() { scope.launch {
        loading = true; failed = false
        try { app.loadWorkspaceEmojis(true) }
        catch (e: CancellationException) { throw e } catch (_: Exception) { failed = true }
        finally { loading = false }
    } }
    LaunchedEffect(app) { load() }
    DisposableEffect(Unit) { onDispose { file?.let { app.platform.action("fileRelease:${it.text("id")}") } } }
    SettingsSection(app.t("home.customEmojis"), app.t("customEmoji.hint")) {
        Field(name, app.t("customEmoji.name"), Modifier.fillMaxWidth(), enabled = !busy) { name = it }
        Field(category, app.t("customEmoji.category"), Modifier.fillMaxWidth(), enabled = !busy) { category = it }
        Field(aliases, app.t("customEmoji.aliases"), Modifier.fillMaxWidth(), enabled = !busy) { aliases = it }
        Button(app.t("customEmoji.image"), enabled = !busy) { scope.launch {
            try {
                val picked = json.parseToJsonElement(app.platform.invoke("pickFiles", buildJsonObject {
                    put("owner", "custom-emojis"); put("accept", "image/png,image/jpeg,image/gif,image/webp"); put("multiple", false)
                }.toString())).jsonObject.array("files").rows().firstOrNull()
                if (picked != null) { file?.let { app.platform.action("fileRelease:${it.text("id")}") }; file = picked }
            } catch (e: CancellationException) { throw e } catch (_: Exception) { failed = true }
        } }
        file?.let { Text(it.text("name")) }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(app.t("home.save"), enabled = !busy && Regex("^[a-z0-9_-]{1,64}$").matches(name) && (file != null || previous != null)) {
                scope.launch {
                    busy = true; failed = false
                    try {
                        val selected = file
                        if (selected != null) require((selected.text("size").toLongOrNull() ?: 0) in 1L..(2L * 1024 * 1024))
                        val payload = buildJsonObject {
                            put("name", name); put("category", category.trim())
                            put("aliases", JsonArray(aliases.split(',').map { it.trim() }.filter { it.isNotEmpty() }.distinct().map(::JsonPrimitive)))
                            previous?.let { put("previous_name", it.text("name")) }
                            if (selected != null) { put("content_type", selected.text("type")); put("data", app.platform.invoke("fileRead", selected.text("id"))) }
                            else put("url", previous!!.text("url"))
                        }
                        app.api.request(if (selected == null) "/api/emojis" else "/api/emojis/upload", "POST", payload)
                        reset(); app.loadWorkspaceEmojis(true)
                    } catch (e: CancellationException) { throw e } catch (_: Exception) { failed = true }
                    finally { busy = false }
                }
            }
            if (previous != null) Button(app.t("customEmoji.cancel"), enabled = !busy) { reset() }
        }
        if (loading) Text(app.t("home.loading"), Modifier.then(webControlModifier("status")))
        if (failed) { Text(app.t("home.controlsError"), Modifier.then(webControlModifier("alert"))); Button(app.t("home.refresh")) { load() } }
        if (!loading && app.workspaceEmojis.isEmpty()) Text(app.t("home.noEmojis"))
        app.workspaceEmojis.groupBy { it.text("category") }.forEach { (group, items) ->
            Text(group.ifEmpty { app.t("customEmoji.uncategorized") })
            items.forEach { item ->
                FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    ActivityPubEmoji(workspaceEmoji(item), size = 32.dp)
                    Text(":${item.text("name")}:")
                    Text(item.array("aliases").joinToString(", ") { it.jsonPrimitive.content })
                    Button(app.t("customEmoji.edit"), enabled = !busy) {
                        reset(); previous = item; name = item.text("name"); category = item.text("category")
                        aliases = item.array("aliases").joinToString(", ") { it.jsonPrimitive.content }
                    }
                    Button(app.t("customEmoji.remove"), enabled = !busy) { remove = item }
                }
            }
        }
    }
    remove?.let { item -> ConfirmationPanel(app.t("customEmoji.remove"), ":${item.text("name")}:", app.t("customEmoji.remove"), busy, if (failed) app.t("home.controlsError") else null, { remove = null }) {
        scope.launch {
            busy = true; failed = false
            try {
                app.api.request("/api/emojis/${item.text("name")}", "DELETE")
                if (previous == item) reset()
                remove = null; app.loadWorkspaceEmojis(true)
            } catch (e: CancellationException) { throw e } catch (_: Exception) { failed = true }
            finally { busy = false }
        }
    } }
}
