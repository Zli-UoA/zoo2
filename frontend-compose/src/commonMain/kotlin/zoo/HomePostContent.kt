package zoo

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import io.ktor.http.encodeURLParameter
import kotlinx.serialization.json.JsonPrimitive

/** Strip only the receiver's appended quote link, retaining links in the user's comment. */
internal fun homePostBody(content: String, quote: Row?): String {
    val uri = quote?.text("uri").orEmpty()
    if (uri.isEmpty()) return content
    val link = "<$uri>"
    return when {
        content == link -> ""
        content.endsWith("\n\n$link") -> content.removeSuffix("\n\n$link")
        else -> content
    }
}

internal fun homeQuoteDestination(quote: Row): String? {
    val post = quote["post"] as? Row
    if (post != null && post["remote"] == JsonPrimitive(false) && post.text("id").isNotBlank())
        return "/home/posts/${post.text("id").encodeURLParameter()}"
    if (post == null && quote.flag("local")) return null
    return publicUrl(quote.text("uri"))?.toString()
}

/** Shared by timelines, reply context and public profiles; quotes never fetch another post. */
@Composable
fun HomePostContent(app: Session, post: Row, previewLinks: Boolean = true) {
    val quote = post["quote"] as? Row
    val body = remember(post.text("content"), quote) { homePostBody(post.text("content"), quote) }
    Column(Modifier.fillMaxWidth().clipToBounds(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (body.isNotBlank()) ActivityPubContent(body, post.homeRows("emojis"),
            modifier = Modifier.fillMaxWidth(), textStyle = MaterialTheme.typography.bodyMedium)
        if (previewLinks) LinkPreviews(app, body, excludeUrl = quote?.text("uri"), showFallback = false)
        if (quote != null) HomeQuoteCard(app, quote)
        MessageAttachments(app, post.homeRows("attachments"))
        if (post.homeRows("attachments").isEmpty()) HomeMedia(post.homeRows("media"))
    }
}

@Composable
private fun HomeQuoteCard(app: Session, quote: Row) {
    val post = quote["post"] as? Row
    val destination = homeQuoteDestination(quote)
    Column(Modifier.fillMaxWidth().border(1.dp, MaterialTheme.colorScheme.outlineVariant,
        RoundedCornerShape(12.dp)).padding(12.dp).then(webControlModifier("group",
        attributes = mapOf("aria-label" to app.t("home.quotedPost")))),
        verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (post == null) {
            Text(app.t("home.quoteUnavailable"), muted = true)
            if (destination != null) NativeLink(destination) {
                Text(app.t("home.openQuotedPost"), color = LocalLinkColor.current)
            }
        } else {
            val name = post.text("display_name").ifEmpty { post.text("user_name") }
            val header: @Composable () -> Unit = {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (post.flag("remote")) ActivityPubAvatar(post["avatar"] as? Row, name)
                    else Avatar(post)
                    Column(Modifier.weight(1f)) {
                        ActivityPubText(name, post.homeRows("actor_emojis"),
                            textStyle = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.SemiBold))
                        Text("@${post.text("user_name")}", small = true, muted = true)
                    }
                }
            }
            if (destination != null) NativeLink(destination, Modifier.fillMaxWidth()) { header() } else header()
            // Match Web: one quote level, no Summaly inside a quote.
            HomePostContent(app, kotlinx.serialization.json.JsonObject(post - "quote"), previewLinks = false)
            val time: @Composable () -> Unit = {
                PostTimestamp(app, post.text("created_at"))
            }
            if (destination != null) NativeLink(destination) { time() } else time()
        }
    }
}
