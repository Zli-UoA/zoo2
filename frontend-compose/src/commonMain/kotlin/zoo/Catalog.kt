package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*

/** Samples live beside the shared UI so native catalogs can render the same examples. */
object CatalogSamples {
    const val message = "こんにちは。今日のリリースは 18:00 です。詳細は https://example.com/docs を参照してください。"
    const val richText =
        "**Compose Multiplatform**\n\n$[tada Hello!] :smile: **太字**\n\n\\[x^2 + y^2 = z^2\\]\n\n```kotlin\nval greeting = \"こんにちは\"\n```"
    val users = listOf(obj("id" to "alice", "display_name" to "Alice", "user_name" to "alice"))
    val channel = obj("id" to "catalog", "channel_name" to "general", "created_user_id" to "alice")
}

@Composable
fun Catalog(app: Session, options: Row) {
    val kind = options.text("sample")
    val fixture = options.nested("fixture")
    val users = (fixture["users"] as? JsonArray)?.map { it.jsonObject } ?: CatalogSamples.users

    val content =
        if ("content" in options) options.text("content")
        else if (kind == "rich-text") CatalogSamples.richText else CatalogSamples.message
    var selected by remember { mutableStateOf("") }
    var picker by remember(kind) { mutableStateOf(kind == "emoji-picker") }
    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        Column(
            Modifier.fillMaxSize()
                .padding(
                    vertical =
                        if (
                            kind in
                                listOf(
                                    "settings",
                                    "settings-members",
                                    "bubble",
                                    "notifications",
                                    "notifications-loading",
                                )
                        )
                            0.dp
                        else 24.dp
                )
        ) {
            when (kind) {
                "card" -> MessageCard(fixture.nested("card"), fixture.flag("attachment"))
                "app-blocks" ->
                    Blocks(
                        app,
                        fixture["blocks"],
                        fixture.text("appId"),
                        submit = fixture.flag("submit"),
                    )
                "link-preview" -> LinkPreviews(app, content)

                "settings",
                "settings-members" -> ZooSettings(app, users)
                "bubble" -> BubbleChat(app, users)
                "notifications",
                "notifications-loading" ->
                    Notifications(
                        app,
                        (fixture["notifications"] as? JsonArray)?.map { it.jsonObject }
                            ?: listOf(
                                obj(
                                    "id" to "catalog-notification",
                                    "actor_id" to "alice",
                                    "user_id" to "alice",
                                    "channel_id" to "catalog",
                                    "message_id" to "catalog-message",
                                    "notification_type" to "mention",
                                    "created_at" to "2026-01-02T09:00:00Z",
                                    "is_read" to "true",
                                )
                            ),
                        users,
                    )
                "composer" -> Composer(app, "catalog", null, users, !options.flag("disabled"))
                "emoji-picker" ->
                    Column(Modifier.padding(16.dp)) {
                        Button(app.t("reactions.add")) { picker = true }
                        Text(selected)
                    }
                "rich-text" ->
                    Column(Modifier.fillMaxWidth().padding(20.dp)) { RichText(content, users) }
                else -> {
                    val message =
                        JsonObject(
                            obj(
                                "id" to "catalog-message",
                                "user_id" to "alice",
                                "content" to content,
                                "created_at" to "2026-01-02T09:00:00Z",
                            ) + options.nested("fixture")
                        )
                    val attachmentRows =
                        (options.nested("fixture")["attachments"] as? JsonArray)
                            ?.map { it.jsonObject }
                            .orEmpty()
                    CompositionLocalProvider(LocalAttachments provides attachmentRows) {
                        Message(
                            app,
                            CatalogSamples.channel,
                            message,
                            listOf(message) +
                                List(fixture.text("replyCount").toIntOrNull() ?: 0) { index ->
                                    obj("id" to "reply-$index", "parent_id" to message.text("id"))
                                },
                            fixture.array("reactions").map { it.jsonObject },
                            users,
                            options.flag("grouped"),
                            true,
                        )
                    }
                }
            }
            app.toast?.let { Text(it, Modifier.padding(16.dp)) }
        }
        if (picker) EmojiPicker(app, { picker = false }) { selected = it }
        OverlayHost(app)
    }
}
