package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.CancellationException

@Composable
fun HomeReactionPicker(app: Session, post: Row, close: () -> Unit, select: (Row) -> Unit) {
    var custom by remember { mutableStateOf(post.flag("remote")) }
    var fullPicker by remember { mutableStateOf(false) }
    var emojis by remember { mutableStateOf<List<Row>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var failed by remember { mutableStateOf(false) }
    var retry by remember { mutableStateOf(0) }
    var query by remember { mutableStateOf("") }
    LaunchedEffect(post.text("id"), retry) {
        loading = true
        failed = false
        try {
            emojis =
                (app.api.request(
                        "/api/activitypub/reaction-options?post_id=${post.text("id").encodeURLParameter()}"
                    ) as Row)
                    .homeRows("emojis")
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            failed = true
        } finally {
            loading = false
        }
    }
    if (fullPicker) {
        EmojiPicker(app, { fullPicker = false }) {
            select(obj("reaction" to it))
            close()
        }
        return
    }
    Panel(app.t("home.react"), close, 448.dp) {
        Row(Modifier.fillMaxWidth().then(webControlModifier("tablist"))) {
            listOf(false to "reactions.tabEmoji", true to "home.customEmojis").forEach {
                (value, key) ->
                Button(
                    app.t(key),
                    modifier = Modifier.weight(1f),
                    selected = custom == value,
                    role = androidx.compose.ui.semantics.Role.Tab,
                ) {
                    custom = value
                }
            }
        }
        if (!custom) {
            FlowRow(
                Modifier.then(
                    webControlModifier(
                        "group",
                        attributes = mapOf("aria-label" to app.t("home.quickReactions")),
                    )
                ),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                listOf("❤️", "👍", "🎉", "😊", "😢").forEach { emoji ->
                    Button(emoji, size = "lg") {
                        select(obj("reaction" to emoji))
                        close()
                    }
                }
            }
            EmojiPicker(app, close, embedded = true) {
                select(obj("reaction" to it))
                close()
            }
        } else {
            Field(query, app.t("home.searchEmoji"), Modifier.fillMaxWidth()) { query = it }
            if (loading) Text(app.t("home.loading"))
            if (failed) {
                Text(app.t("home.controlsError"), Modifier.then(webControlModifier("alert")))
                Button(app.t("home.refresh")) { retry++ }
            }
            val filtered = emojis.filter { it.text("name").contains(query, ignoreCase = true) }
            if (!loading && !failed && filtered.isEmpty()) Text(app.t("home.noEmojis"))
            FlowRow(
                Modifier.fillMaxWidth()
                    .heightIn(max = 320.dp)
                    .verticalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                filtered.take(200).forEach { emoji ->
                    Box(
                        Modifier.size(40.dp)
                            .then(
                                webControlModifier(
                                    "button",
                                    attributes = mapOf("aria-label" to emoji.text("name")),
                                )
                            )
                            .clickable {
                                select(
                                    obj(
                                        "reaction" to emoji.text("name"),
                                        "emoji_id" to emoji.text("image_id"),
                                    )
                                )
                                close()
                            },
                        contentAlignment = Alignment.Center,
                    ) {
                        ActivityPubEmoji(emoji, 28.dp)
                    }
                }
            }
            if (filtered.size > 200) Text(app.t("home.emojiSearchMore"))
        }
    }
}
