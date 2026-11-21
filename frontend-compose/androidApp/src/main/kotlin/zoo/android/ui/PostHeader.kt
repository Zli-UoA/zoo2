package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal enum class PostAudience(val icon: UiIcon, val label: String) {
    Public(UiIcon.Public, "公開"),
    Unlisted(UiIcon.Unlisted, "未収載"),
    Followers(UiIcon.Followers, "フォロワー限定"),
    Local(UiIcon.Home, "ローカル"),
    Direct(UiIcon.Lock, "宛先限定");

    companion object {
        fun fromApi(visibility: String): PostAudience? = when (visibility) {
            "public" -> Public
            "unlisted" -> Unlisted
            "followers" -> Followers
            "local" -> Local
            "direct" -> Direct
            else -> null
        }
    }
}

internal data class PostHeaderData(
    val handle: String?,
    val dateTime: String,
    val audience: PostAudience?,
    val channel: String?,
    val server: String?,
    val remoteServer: Boolean = false,
    val serverRgb: Int? = null,
)

@Composable
internal fun PostHeader(author: String, data: PostHeaderData, colors: AndroidColors) {
    Column(Modifier.fillMaxWidth().padding(bottom = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(author, Modifier.weight(1f, fill = false), color = colors.foreground,
                    fontSize = entryTextSize, fontWeight = AndroidWeights.entryAuthor, maxLines = 1, overflow = TextOverflow.Ellipsis)
                data.handle?.let {
                    Text(it, Modifier.widthIn(max = 72.dp), color = colors.secondary,
                        fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            Text(data.dateTime, color = colors.secondary, fontSize = 11.sp, maxLines = 1)
            data.audience?.let { audience ->
                Box(Modifier.semantics { contentDescription = audience.label }) {
                    UiGlyph(audience.icon, colors.secondary, iconSize = AndroidControlSize.metadataIcon)
                }
            }
        }
        val sourceFontSize = 11.sp
        val sourceLineHeight = 14.sp
        val sourceHeight = with(LocalDensity.current) { sourceLineHeight.toDp() }
        if (data.channel != null) {
            Text("#${data.channel}", Modifier.height(sourceHeight), color = colors.secondary, fontSize = sourceFontSize, lineHeight = sourceLineHeight,
                fontWeight = AndroidWeights.label, maxLines = 1, overflow = TextOverflow.Ellipsis)
        } else data.server?.let { server ->
            val tint = data.serverRgb?.let { Color(it or 0xff000000.toInt()) }
            val start = tint?.let { lerp(colors.background, it, 0.32f) } ?: colors.headerLight
            Row(Modifier.fillMaxWidth()
                .background(Brush.horizontalGradient(listOf(start, Color.Transparent)), RoundedCornerShape(5.dp))
                .padding(horizontal = 6.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                UiGlyph(UiIcon.Public, colors.foreground, iconSize = AndroidControlSize.metadataIcon)
                Text(server, Modifier.height(sourceHeight), color = colors.foreground, fontSize = sourceFontSize, lineHeight = sourceLineHeight, fontWeight = AndroidWeights.emphasis,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}
