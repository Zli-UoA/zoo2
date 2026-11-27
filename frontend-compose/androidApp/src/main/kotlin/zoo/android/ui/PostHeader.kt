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
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.TextUnit
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
        Direct -> label
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

/** Keep the full timestamp and its status icon together, leaving room for the author. */
@Composable
internal fun AuthorTimeHeader(
    time: String,
    colors: AndroidColors,
    timeSize: TextUnit = 12.sp,
    alignTimeToEnd: Boolean = false,
    keepFullAuthor: Boolean = false,
    status: (@Composable () -> Unit)? = null,
    author: @Composable () -> Unit,
) {
    Layout(content = {
        Box { author() }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(time, Modifier.weight(1f, fill = false), color = colors.secondary, fontSize = timeSize)
            status?.invoke()
        }
    }) { (name, metadata), constraints ->
        val width = constraints.maxWidth
        val gap = 8.dp.roundToPx()
        val loose = constraints.copy(minWidth = 0, minHeight = 0)
        val meta = metadata.measure(loose)
        val remaining = width - meta.width - gap
        val stack = (keepFullAuthor || remaining < AndroidControlSize.target.roundToPx()) &&
            remaining < name.maxIntrinsicWidth(Constraints.Infinity)
        val label = name.measure(loose.copy(maxWidth = if (stack) width else remaining))
        val height = if (stack) label.height + meta.height else maxOf(label.height, meta.height)
        layout(width, height) {
            label.placeRelative(0, if (stack) 0 else (height - label.height) / 2)
            val x = when {
                stack -> 0
                alignTimeToEnd -> width - meta.width
                else -> label.width + gap
            }
            meta.placeRelative(x, if (stack) label.height else (height - meta.height) / 2)
        }
    }
}

@Composable
internal fun PostHeader(author: String, data: PostHeaderData, colors: AndroidColors, openProfile: (() -> Unit)? = null) {
    val profileAction = if (openProfile == null) Modifier else Modifier.clickable(
        onClickLabel = androidLabel("home.openAuthorProfile", "「${author}」のプロフィールを開く", "author" to author),
        role = Role.Button, onClick = openProfile,
    )
    Column(Modifier.fillMaxWidth().padding(bottom = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        AuthorTimeHeader(data.dateTime, colors, timeSize = 11.sp, alignTimeToEnd = true, status = {
            data.audience?.let { audience ->
                val label = if (audience == PostAudience.Followers)
                    androidLabel("home.visibility.followers", "フォロワー限定") else audience.localizedLabel()
                Box(Modifier.semantics { contentDescription = label }) {
                    UiGlyph(audience.icon, colors.secondary, iconSize = AndroidControlSize.metadataIcon)
                }
            }
        }) {
            Row(profileAction, verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(author, Modifier.weight(1f, fill = false), color = colors.foreground,
                    fontSize = entryTextSize, fontWeight = AndroidWeights.entryAuthor, maxLines = 1, overflow = TextOverflow.Ellipsis)
                data.handle?.let {
                    Text(it, Modifier.widthIn(max = 72.dp), color = colors.secondary,
                        fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
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
