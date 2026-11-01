package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*

@Composable
fun PreferenceStory(app: Session, options: Row) {
    val fixture = options.nested("fixture")
    val kind = fixture.text("kind")
    val state = fixture.text("state")
    val users = fixture.array("users").map { it.jsonObject }
    LaunchedEffect(kind, state) {
        if (kind == "pwa" || kind == "settings") app.capabilities = fixture.nested("capabilities")
    }
    when (kind) {
        "settings" -> ZooSettings(app, users)
        "members" -> WorkspaceMembers(app, users)
        "slackimport" -> SlackImportPanel(app)
        // Storybook では Slack へ移動せず、Story の navigateTo を呼ぶ
        "slacklink" ->
            SlackLinkPanel(app) { url ->
                if (fixture.array("callbacks").any { it.jsonPrimitive.content == "navigateTo" })
                    app.platform.invoke(
                        "storyCallback",
                        obj("name" to "navigateTo", "args" to JsonArray(listOf(JsonPrimitive(url))))
                            .toString(),
                    )
            }
        "mfmsearch" ->
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopEnd) {
                    SearchEngineSelect(app)
                }
                if (state == "Long Query Mobile") SearchQuery(app, fixture.text("longSearchQuery"))
                SearchQuery(app, fixture.text("searchQuery"))
            }
        "theme" ->
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ThemeToggle(app)
            }
        "language" ->
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                LanguageToggle(app, fixture.text("presentation"), false)
            }
        "pwa" ->
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                PwaControls(app, fixture.text("presentation"))
            }
        else -> error("Unknown preference story: $kind")
    }
}
