package zoo.android.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import zoo.ChannelLinkTarget

/** このアプリのチャンネル・メッセージへのリンクの、本文中の短い表示と埋め込みカードに出す内容。 */
internal sealed interface ChannelLinkView {
    /** 取得中。カードの形だけを置く。 */
    data class Loading(val label: String) : ChannelLinkView

    /** 見られない・読み込めない。押せない。 */
    data class Notice(val icon: UiIcon, val text: String) : ChannelLinkView

    data class Channel(
        /** `#名前`（DM は総称）。 */
        val channel: String,
        /** 非公開・暗号化の説明。 */
        val note: String?,
        val description: String,
    ) : ChannelLinkView

    data class Message(
        /** 本文中の短い表示（「○○さん: #チャンネル名」）。 */
        val inlineLabel: String,
        val author: String,
        val time: String,
        /** `#名前`。返信は「#名前 内のスレッド」。 */
        val channel: String,
        val body: ChannelLinkBody?,
        /** 返信なら、そのスレッドの親。 */
        val parentId: String?,
    ) : ChannelLinkView
}

/** カードの本文。`icon` があるものは、本文の代わりの説明。 */
internal data class ChannelLinkBody(val text: String, val icon: UiIcon? = null, val italic: Boolean = false)

/**
 * 画面側が渡す、アプリ内リンクの解釈と移動。[view] は、呼んだ行が画面に出ている間だけ行き先を取得する
 * （`LazyColumn` は画面の近くの行だけを作る）。
 */
internal class ChannelLinkProvider(
    /** このアプリのチャンネル・メッセージへのリンクなら行き先。 */
    val target: (String) -> ChannelLinkTarget?,
    val view: @Composable (ChannelLinkTarget) -> ChannelLinkView,
    /** アプリ内で移動する。`parentId` は、分かっていればリンク先の返信の親。 */
    val open: (ChannelLinkTarget, String?) -> Unit,
)

internal val LocalChannelLinks = staticCompositionLocalOf<ChannelLinkProvider?> { null }

@Composable
internal fun ChannelLinkCard(url: String, colors: AndroidColors, menu: (() -> Unit)? = null) {
    val links = LocalChannelLinks.current ?: return
    val target = remember(url, links) { links.target(url) } ?: return
    key(target) {
        val view = links.view(target)
        ChannelLinkCardContent(view, colors, menu) {
            links.open(target, (view as? ChannelLinkView.Message)?.parentId)
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun ChannelLinkCardContent(
    view: ChannelLinkView, colors: AndroidColors, menu: (() -> Unit)? = null, open: () -> Unit,
) {
    val surface = Modifier.padding(vertical = 4.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp))
        .background(colors.inputSurface)
    val pressable = surface.combinedClickable(role = Role.Button, onClick = open, onLongClick = menu)
        .heightIn(min = AndroidControlSize.target).padding(12.dp)
    when (view) {
        is ChannelLinkView.Loading -> Box(
            // 読み込み中は、行の高さが後から大きく動かないよう、カードの形だけを置く。
            surface.height(84.dp).semantics { contentDescription = view.label },
        )
        is ChannelLinkView.Notice -> Row(
            surface.padding(12.dp), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            UiGlyph(view.icon, colors.secondary, AndroidControlSize.inlineIcon)
            Text(view.text, color = colors.secondary, fontSize = 14.sp)
        }
        is ChannelLinkView.Channel -> Column(pressable, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(view.channel, color = colors.foreground, fontSize = 15.sp, fontWeight = AndroidWeights.emphasis,
                maxLines = 1, overflow = TextOverflow.Ellipsis)
            view.note?.let { LinkNote(ChannelLinkBody(it, UiIcon.Lock), colors) }
            if (view.description.isNotBlank()) {
                Text(view.description, color = colors.secondary, fontSize = 14.sp, maxLines = 2,
                    overflow = TextOverflow.Ellipsis)
            }
        }
        is ChannelLinkView.Message -> Row(pressable, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.size(24.dp).background(colors.background, CircleShape), contentAlignment = Alignment.Center) {
                Text(view.author.take(1), color = colors.secondary, fontSize = 12.sp)
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(view.author, Modifier.weight(1f, fill = false), color = colors.foreground, fontSize = 14.sp,
                        fontWeight = AndroidWeights.entryAuthor, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(view.time, color = colors.secondary, fontSize = 12.sp, maxLines = 1)
                }
                Text(view.channel, color = colors.secondary, fontSize = 12.sp, maxLines = 1,
                    overflow = TextOverflow.Ellipsis)
                view.body?.let { body ->
                    if (body.icon != null) LinkNote(body, colors)
                    else Text(body.text, color = colors.foreground, fontSize = 14.sp, maxLines = 3,
                        overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
}

@Composable
private fun LinkNote(note: ChannelLinkBody, colors: AndroidColors) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        note.icon?.let { UiGlyph(it, colors.secondary, AndroidControlSize.metadataIcon) }
        Text(note.text, color = colors.secondary, fontSize = 14.sp,
            fontStyle = if (note.italic) FontStyle.Italic else FontStyle.Normal)
    }
}
