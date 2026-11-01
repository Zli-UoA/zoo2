package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

@Composable
fun AppConnections(
    app: Session,
    appId: String,
    channelId: String?,
    connections: Boolean,
    assistant: Boolean,
) {
    if (!connections && !assistant) return
    var connection by remember(appId) { mutableStateOf<String?>(null) }
    var sessions by remember(appId) { mutableStateOf<List<Row>>(emptyList()) }
    var error by remember(appId) { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var revision by remember { mutableStateOf(0) }
    LaunchedEffect(appId, connections, revision) {
        if (connections)
            while (isActive) {
                try {
                    connection =
                        app.api.request("/api/apps/$appId/connection").jsonObject.text("state")
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    error = app.t("apps.loadError")
                }
                delay(2000)
            }
    }
    LaunchedEffect(appId, assistant, revision) {
        if (assistant)
            while (isActive) {
                try {
                    sessions =
                        app.api.request("/api/assistant-sessions").rows().filter {
                            it.text("app_id") == appId
                        }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    error = app.t("apps.loadError")
                }
                delay(2000)
            }
    }
    LaunchedEffect(appId, assistant, channelId) {
        if (assistant && !channelId.isNullOrEmpty())
            try {
                app.api.request(
                    "/api/apps/$appId/context",
                    "POST",
                    obj(
                        "channel_id" to channelId,
                        "context" to obj("view" to "app", "channel_id" to channelId),
                    ),
                )
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {}
    }
    fun run(path: String, body: JsonElement, method: String = "POST") {
        if (busy) return
        app.scope.launch {
            busy = true
            error = ""
            try {
                app.api.request(path, method, body)
                revision++
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                error = app.t("apps.actionError")
            } finally {
                busy = false
            }
        }
    }
    Column(
        Modifier.fillMaxWidth()
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(10.dp))
            .padding(13.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (connections)
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    app.t(
                        if (connection == null) "apps.loading"
                        else
                            "apps.userConnection.${if(connection == "connected") "connected" else if(connection == "requested") "requested" else "disconnected"}"
                    )
                )
                Button(
                    app.t(
                        "apps.userConnection.${if(connection == "disconnected") "connect" else "disconnect"}"
                    ),
                    !busy && connection != null,
                    variant = "outline",
                ) {
                    run(
                        "/api/apps/$appId/connection",
                        obj("connect" to (connection == "disconnected")),
                    )
                }
            }
        if (assistant && !channelId.isNullOrEmpty())
            Button(
                app.t("apps.assistant.start"),
                !busy && app.config.userId.isNotEmpty(),
                variant = "outline",
            ) {
                run(
                    "/api/assistant-sessions",
                    obj(
                        "app_id" to appId,
                        "user_id" to app.config.userId,
                        "channel_id" to channelId,
                        "title" to app.t("apps.assistant.session"),
                        "context" to obj("channel_id" to channelId),
                    ),
                )
            }
        if (assistant)
            sessions
                .filter { it.text("state") == "running" }
                .forEach { session ->
                    var title by
                        remember(session.text("id")) { mutableStateOf(session.text("title")) }
                    Row(
                        Modifier.fillMaxWidth()
                            .background(
                                MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .4f),
                                RoundedCornerShape(10.dp),
                            )
                            .padding(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Field(
                            title,
                            app.t("apps.assistant.title"),
                            modifier = Modifier.weight(1f),
                            onBlur = {
                                if (title != session.text("title"))
                                    run(
                                        "/api/assistant-sessions/${session.text("id")}",
                                        obj("title" to title),
                                        "PATCH",
                                    )
                            },
                            showLabel = false,
                            enabled = !busy,
                        ) {
                            title = it.take(200)
                        }
                        Button(app.t("apps.assistant.stop"), !busy) {
                            run(
                                "/api/assistant-sessions/${session.text("id")}",
                                obj("state" to "stopped"),
                                "PATCH",
                            )
                        }
                    }
                }
        if (error.isNotEmpty())
            Text(
                error,
                Modifier.then(webControlModifier("alert")).semantics {
                    liveRegion = LiveRegionMode.Polite
                },
                color = MaterialTheme.colorScheme.error,
            )
    }
}
