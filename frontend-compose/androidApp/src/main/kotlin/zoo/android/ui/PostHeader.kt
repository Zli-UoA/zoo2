package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal enum class PostAudience(val apiValue: String, val icon: UiIcon, val label: String) {
    Public("public", UiIcon.Public, "パブリック"),
    Unlisted("unlisted", UiIcon.Home, "ホーム"),
    Followers("followers", UiIcon.Lock, "フォロワー"),
    Local("local", UiIcon.CloudOff, "ローカルのみ"),
    Direct("direct", UiIcon.Lock, "宛先限定");

    @Composable
    fun localizedLabel(): String = when (this) {
        Followers -> androidLabel("home.followersAudience", label)
        Direct -> androidLabel("android.post.visibility.direct", label)
        else -> androidLabel("home.visibility.$apiValue", label)
    }

    companion object {
        val composerOptions = listOf(Public, Unlisted, Followers)

        fun fromApi(visibility: String): PostAudience? = entries.firstOrNull { it.apiValue == visibility }
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
internal fun PostHeader(author: String, data: PostHeaderData, colors: AndroidColors, openProfile: (() -> Unit)? = null) {
    val profileAction = if (openProfile == null) Modifier else Modifier.clickable(
        onClickLabel = androidLabel("home.openAuthorProfile", "「${author}」のプロフィールを開く", "author" to author),
        role = Role.Button, onClick = openProfile,
    )
    Column(Modifier.fillMaxWidth().padding(bottom = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(Modifier.weight(1f).then(profileAction), verticalAlignment = Alignment.CenterVertically,
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
                val label = if (audience == PostAudience.Followers)
                    androidLabel("home.visibility.followers", "フォロワー限定") else audience.localizedLabel()
                Box(Modifier.semantics { contentDescription = label }) {
                    UiGlyph(audience.icon, colors.secondary, iconSize = AndroidControlSize.metadataIcon)
                }
            }
        }
        val channel = data.channel
        val source = channel?.let { "#$it" } ?: data.server
        if (source != null) {
            val server = channel == null
            val sourceLineHeight = 14.sp
            val sourceHeight = with(LocalDensity.current) { sourceLineHeight.toDp() }
            val start = if (server) {
                val tint = data.serverRgb?.let { Color(it or 0xff000000.toInt()) }
                tint?.let { lerp(colors.background, it, 0.32f) } ?: colors.headerLight
            } else colors.inputSurface
            Row(
                Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.metadataIcon)
                    .background(Brush.horizontalGradient(listOf(start, Color.Transparent)), RoundedCornerShape(5.dp))
                    .padding(horizontal = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                if (server) UiGlyph(UiIcon.Public, colors.foreground, iconSize = AndroidControlSize.metadataIcon)
                Text(source, Modifier.height(sourceHeight),
                    color = if (server) colors.foreground else colors.secondary,
                    fontSize = 11.sp, lineHeight = sourceLineHeight,
                    fontWeight = if (server) AndroidWeights.emphasis else AndroidWeights.label,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}
