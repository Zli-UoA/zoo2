package zoo

import androidx.compose.foundation.clickable
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.collect

@Composable
fun ActivityPubEmoji(
    emoji: Row,
    size: Dp = 20.dp,
    modifier: Modifier = Modifier,
    onFailure: (() -> Unit)? = null,
) {
    val app = LocalApp.current
    val id = emoji.text("image_id")
    var failed by remember(id) { mutableStateOf(!emoji.flag("local") && !Regex("^[a-f0-9]{64}$").matches(id)) }
    var bitmap by remember(id) { mutableStateOf<androidx.compose.ui.graphics.ImageBitmap?>(null) }
    LaunchedEffect(id) {
        if (!failed)
            try {
                val bytes =
                    app.image(if (emoji.flag("local")) { val url = emoji.text("url"); if (url.startsWith("/")) endpoint(app.config.apiBaseUrl, url) else url } else endpoint(app.config.apiBaseUrl, "/api/activitypub/emoji-images/$id"))
                app.platform.imageFrames(bytes).collect { bitmap = it; app.emojiRatios[id] = (it.width.toFloat() / it.height).coerceIn(0.1f, 8f) }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                failed = true
                onFailure?.invoke()
            }
        else onFailure?.invoke()
    }
    if (failed) Text(emoji.text("name"), modifier)
    else
        bitmap?.let { Image(it, emoji.text("name"), modifier.height(size).width(size * (app.emojiRatios[id] ?: 1f))) }
            ?: Box(modifier.size(size))
}

internal val LocalActivityPubEmojis = staticCompositionLocalOf<List<Row>> { emptyList() }

@Composable
fun ActivityPubContent(text: String, emojis: List<Row> = emptyList(), modifier: Modifier = Modifier,
    textStyle: TextStyle = TextStyle.Default, maxLines: Int = Int.MAX_VALUE) {
    CompositionLocalProvider(LocalActivityPubEmojis provides emojis,
        LocalMfmStyle provides LocalMfmStyle.current.merge(textStyle)) {
        Box(modifier) { RichText(text, maxLines = maxLines) }
    }
}

@Composable
fun ActivityPubText(
    text: String,
    emojis: List<Row> = emptyList(),
    modifier: Modifier = Modifier,
    textStyle: TextStyle? = null,
    maxLines: Int = Int.MAX_VALUE,
) {
    val failed = remember(emojis) { mutableStateMapOf<String, Boolean>() }
    val fonts = LocalFonts.current
    val style = MaterialTheme.typography.bodyMedium.merge(LocalMfmStyle.current).merge(textStyle)
    val density = LocalDensity.current
    val size = with(density) { (if (style.fontSize.isSp) style.fontSize else 14.sp).toDp() * 1.3f }
    val parts = mutableMapOf<String, Row>()
    val annotated = buildAnnotatedString {
        var cursor = 0
        Regex(":[A-Za-z0-9_+\\-]+:").findAll(text).forEach { match ->
            append(fontText(text.substring(cursor, match.range.first), fonts, style.fontWeight))
            val emoji = emojis.find { it.text("name") == match.value }
            if (
                emoji == null ||
                    failed[emoji.text("image_id")] == true ||
                    (!emoji.flag("local") && !Regex("^[a-f0-9]{64}$").matches(emoji.text("image_id")))
            )
                append(fontText(match.value, fonts, style.fontWeight))
            else {
                val key = "ap-${parts.size}"
                parts[key] = emoji
                appendInlineContent(key, match.value)
            }
            cursor = match.range.last + 1
        }
        append(fontText(text.substring(cursor), fonts, style.fontWeight))
    }
    androidx.compose.material3.Text(
        annotated,
        modifier,
        color =
            style.color.takeIf { it != Color.Unspecified }
                ?: MaterialTheme.colorScheme.onBackground,
        style = style,
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
        inlineContent =
            parts.mapValues { (_, emoji) ->
                InlineTextContent(
                    Placeholder((1.3f * (LocalApp.current.emojiRatios[emoji.text("image_id")] ?: 1f)).em, 1.3.em, PlaceholderVerticalAlign.TextCenter)
                ) {
                    ActivityPubEmoji(
                        emoji,
                        size,
                        onFailure = { failed[emoji.text("image_id")] = true },
                    )
                }
            },
    )
}

@Composable
fun FederatedAccount(
    userName: String,
    actorId: String,
    remote: Boolean = false,
    modifier: Modifier = Modifier,
) {
    val host =
        if (remote)
            try {
                publicUrl(actorId)?.host ?: userName.substringAfterLast('@', "")
            } catch (_: Exception) {
                userName.substringAfterLast('@', "")
            }
        else ""
    val name =
        if (host.isNotEmpty() && userName.endsWith("@$host")) userName.removeSuffix("@$host")
        else userName
    val colors = MaterialTheme.colorScheme
    val fonts = LocalFonts.current
    androidx.compose.material3.Text(
        buildAnnotatedString {
            withStyle(SpanStyle(color = colors.onBackground)) { append(fontText("@$name", fonts)) }
            if (host.isNotEmpty())
                withStyle(SpanStyle(color = colors.onSurfaceVariant)) {
                    append(fontText("@$host", fonts))
                }
        },
        modifier,
        style = MaterialTheme.typography.bodySmall,
    )
}

internal fun Row.homeRows(key: String): List<Row> =
    (get(key) as? kotlinx.serialization.json.JsonArray)?.mapNotNull { it as? Row }.orEmpty()

/** Remote images always use the server cache; malformed IDs keep the ordinary initials. */
@Composable
fun ActivityPubAvatar(avatar: Row?, name: String, modifier: Modifier = Modifier) {
    val app = LocalApp.current
    val id = avatar?.text("image_id").orEmpty()
    val valid = Regex("^[a-f0-9]{64}$").matches(id)
    var bitmap by remember(id) { mutableStateOf<androidx.compose.ui.graphics.ImageBitmap?>(null) }
    var loading by remember(id) { mutableStateOf(valid) }
    LaunchedEffect(id) {
        if (valid)
            try {
                val bytes =
                    app.image(endpoint(app.config.apiBaseUrl, "/api/activitypub/emoji-images/$id"))
                app.platform.imageFrames(bytes).collect {
                    bitmap = it
                    loading = false
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                loading = false
            }
    }
    Box(
        modifier
            .size(32.dp)
            .clip(androidx.compose.foundation.shape.CircleShape)
            .then(
                webControlModifier("img", busy = loading, attributes = mapOf("aria-label" to name))
            )
    ) {
        if (bitmap != null)
            Image(
                bitmap!!,
                null,
                Modifier.fillMaxSize(),
                contentScale = androidx.compose.ui.layout.ContentScale.Crop,
            )
        else Avatar(obj("display_name" to name), size = 32.dp)
    }
}

/** Both authenticated timelines and public profiles use the same image proxy paths. */
@Composable
fun HomeMedia(media: List<Row>) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        media.chunked(2).forEach { pair ->
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                pair.forEach { item -> HomeMediaImage(item, Modifier.weight(1f)) }
                if (pair.size == 1) Spacer(Modifier.weight(1f))
            }
        }
    }
}

@Composable
private fun HomeMediaImage(item: Row, modifier: Modifier) {
    val app = LocalApp.current
    val id = item.text("image_id")
    val path = if (Regex("^[a-f0-9]{64}$").matches(id)) "/api/activitypub/media-images/$id"
        else item.text("url").takeIf { Regex("^/api/home/images/[a-f0-9-]+$").matches(it) }
    if (path == null) return
    val url = endpoint(app.config.apiBaseUrl, path)
    val label = item.text("name").ifEmpty { app.t("home.image") }
    var bitmap by remember(url) { mutableStateOf<androidx.compose.ui.graphics.ImageBitmap?>(null) }
    var failed by remember(url) { mutableStateOf(false) }
    LaunchedEffect(url) {
        try {
            app.platform.imageFrames(app.image(url)).collect { bitmap = it }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) { failed = true }
    }
    val frame = bitmap
    val ratio = frame?.let { it.width.toFloat() / it.height } ?: 1f
    Box(modifier.heightIn(max = 320.dp).aspectRatio(ratio)
        .clip(androidx.compose.foundation.shape.RoundedCornerShape(8.dp))
        .clickable { app.platform.action("open:$url") }) {
        if (frame != null) Image(frame, label, Modifier.fillMaxSize(), contentScale = androidx.compose.ui.layout.ContentScale.Fit)
        else if (failed) Text(label)
    }
}
