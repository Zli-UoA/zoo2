package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.unit.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.serialization.json.*

@Composable
internal fun ThreadChannelsDialog(app: Session, parent: Row) {
    val parentId = parent.text("id")
    var archived by remember(parentId) { mutableStateOf(false) }
    var reload by remember(parentId) { mutableIntStateOf(0) }
    var threads by remember(parentId, archived, reload) { mutableStateOf<List<Row>?>(null) }
    var failed by remember(parentId, archived, reload) { mutableStateOf(false) }
    var creating by remember(parentId) { mutableStateOf(false) }
    var title by remember(parentId) { mutableStateOf("") }
    var firstPost by remember(parentId) { mutableStateOf("") }
    var search by remember(parentId) { mutableStateOf("") }
    val busy = "thread-channel-action" in app.pending

    LaunchedEffect(parentId, archived, reload, search) {
        try {
            delay(250)
            val query = search.trim().takeIf(String::isNotEmpty)
                ?.let { "&search=${it.encodeURLParameter()}" }.orEmpty()
            val response = app.api.request(
                "/api/channels/$parentId/threads?archived=$archived$query",
            ).jsonObject
            threads = response["items"]?.rows().orEmpty()
            failed = false
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            failed = true
        }
    }

    fun runAction(action: suspend () -> Unit) {
        app.run("thread-channel-action") {
            action()
            reload++
        }
    }

    Panel(
        app.t("threads.title"),
        { app.dialog = null },
        512.dp,
        footer = {
            Row(
                Modifier.fillMaxWidth().padding(top = 8.dp),
                horizontalArrangement = Arrangement.End,
            ) {
                Button(
                    if (creating) app.t("common.cancel") else app.t("threads.create"),
                    enabled = !busy,
                    click = { creating = !creating },
                )
            }
        },
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                app.t("threads.active"),
                variant = if (!archived) "tab" else "ghost",
                selected = !archived,
                click = { archived = false },
            )
            Button(
                app.t("threads.archivedList"),
                variant = if (archived) "tab" else "ghost",
                selected = archived,
                click = { archived = true },
            )
        }
        Field(search, app.t("threads.searchPlaceholder"), change = { search = it })
        if (creating) {
            Field(title, app.t("threads.titlePlaceholder"), maxLength = 100, change = { title = it })
            Field(
                firstPost,
                app.t("threads.firstPostPlaceholder"),
                singleLine = false,
                minimumHeight = 88.dp,
                change = { firstPost = it },
            )
            Button(
                app.t("threads.create"),
                primary = true,
                enabled = !busy && title.isNotBlank() && firstPost.isNotBlank(),
                click = {
                    runAction {
                        val result = app.api.request(
                            "/api/channels/$parentId/threads",
                            "POST",
                            buildJsonObject {
                                put("title", title.trim())
                                put("initial_post", firstPost)
                            },
                        ).jsonObject
                        val channelId = result["data"]?.jsonObject?.get("channel_id")?.jsonPrimitive?.content
                            ?: result["channel_id"]?.jsonPrimitive?.content
                        if (channelId != null) {
                            app.dialog = null
                            app.navigate("/channels/$channelId")
                        }
                    }
                },
            )
        }
        when {
            failed -> Button(app.t("offline.retry"), click = { reload++ })
            threads == null -> Text(app.t("common.loading"), muted = true)
            else -> {
                val filtered = threads.orEmpty()
                if (filtered.isEmpty()) {
                    Text(app.t("threads.emptyList"), muted = true)
                } else {
                    Column(
                        Modifier.fillMaxWidth().heightIn(max = 420.dp).verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        filtered.forEach { thread ->
                            val id = thread.text("channel_id")
                            val member = thread.flag("is_member")
                            Row(
                                Modifier.fillMaxWidth()
                                    .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(10.dp))
                                    .padding(12.dp),
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Column(Modifier.weight(1f)) {
                                    Text(thread.text("title"), maxLines = 1)
                                    Text(thread.text("last_message"), small = true, muted = true, maxLines = 1)
                                }
                                if (!member) {
                                    Button(app.t("threads.join"), enabled = !busy, size = "sm") {
                                        runAction {
                                            app.api.request("/api/threads/$id/join", "POST", buildJsonObject {})
                                            app.navigate("/channels/$id")
                                            app.dialog = null
                                        }
                                    }
                                } else {
                                    Button(app.t("threads.open"), size = "sm") {
                                        app.dialog = null
                                        app.navigate("/channels/$id")
                                    }
                                    Button(
                                        app.t(if (archived) "moderation.unarchive" else "threads.archive"),
                                        size = "sm",
                                        enabled = !busy,
                                    ) {
                                        runAction {
                                            app.api.request(
                                                "/api/threads/$id/${if (archived) "open" else "archive"}",
                                                "POST",
                                                buildJsonObject {},
                                            )
                                        }
                                    }
                                    Button(app.t("threads.leave"), size = "sm", enabled = !busy) {
                                        runAction {
                                            app.api.request("/api/threads/$id/leave", "DELETE")
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
