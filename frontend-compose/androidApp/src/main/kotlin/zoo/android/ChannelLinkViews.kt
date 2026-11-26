package zoo.android

import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import kotlinx.coroutines.flow.flowOf
import zoo.ChannelLinkInfo
import zoo.ChannelLinkTarget
import zoo.LinkedChannel
import zoo.LinkedMessage
import zoo.LinkedMessageKind
import zoo.LinkedMessageState
import zoo.Row
import zoo.Session
import zoo.android.ui.ChannelLinkBody
import zoo.android.ui.ChannelLinkProvider
import zoo.android.ui.ChannelLinkView
import zoo.android.ui.UiIcon
import zoo.linkedChannel
import zoo.parseChannelLink
import zoo.resolveChannelLink
import zoo.text

/** 本文のアプリ内リンクを、行き先の状態を購読して表示する [ChannelLinkProvider]。 */
@Composable
internal fun rememberChannelLinks(
    session: Session, platform: AndroidPlatform, open: (ChannelLinkTarget, String?) -> Unit,
): ChannelLinkProvider = remember(session, platform, open) {
    ChannelLinkProvider(
        target = { url -> parseChannelLink(url, session.config.apiBaseUrl) },
        view = { target -> channelLinkState(session, platform, target) },
        open = open,
    )
}

@Composable
private fun channelLinkState(session: Session, platform: AndroidPlatform, target: ChannelLinkTarget): ChannelLinkView {
    val channels by session.repo.channels.rows.collectAsState()
    // 一覧の初回同期（保存した控えではなく、サーバーから）が終わるまで、「見られない」と決めない。
    val synchronized by session.repo.channels.synchronizedRows.collectAsState()
    val users by session.repo.users.rows.collectAsState()
    val usersById = remember(users) { users.associateBy { it.text("id") } }
    val channel = remember(channels, target.channelId) { linkedChannel(channels, target.channelId) }
    // 見られるチャンネルのメッセージだけを取得する。
    val messageId = target.linkedMessageId?.takeIf { channel != null }
    val updates = remember(session, target.channelId, messageId) {
        if (messageId == null) flowOf(LinkedMessageState())
        else session.repo.linkedMessages.observe(target.channelId, messageId)
    }
    val initial = remember(updates) {
        if (messageId == null) LinkedMessageState() else session.repo.linkedMessages.peek(target.channelId, messageId)
    }
    val message by updates.collectAsState(initial)
    val records by remember(session) { session.encryptionRecords() }
        .collectAsState(remember(session) { session.currentEncryptionRecords })
    val info = resolveChannelLink(
        target, channel, directoryReady = synchronized != null, message = message,
        recorded = channel != null && session.isEncryptedChannel(channel.id), records = records,
        users = usersById, hidesContent = { messageRichContent(it).hidesBody },
    )
    return channelLinkView(info, { key, args -> session.t(key, *args) }) { createdAt ->
        channelLinkTime(createdAt, platform.now(), platform::day, platform::time)
    }
}

/** 埋め込みに添える日時。検索結果と同じく「月/日 時:分」で、今年でなければ年も付ける。 */
internal fun channelLinkTime(
    createdAt: String, now: String, day: (String) -> String, time: (String) -> String,
): String {
    val date = day(createdAt)
    val shown = if (date.substringBefore('/') == day(now).substringBefore('/')) date.substringAfter('/') else date
    return listOf(shown, time(createdAt)).filter { it.isNotBlank() }.joinToString(" ")
}

/** 行き先の状態を、画面の言葉にする（Web の `ChannelLinkEmbed`・`channelLinkLabel`）。 */
internal fun channelLinkView(
    info: ChannelLinkInfo,
    t: (String, Array<out Pair<String, String>>) -> String,
    time: (String) -> String,
): ChannelLinkView {
    fun text(key: String, vararg args: Pair<String, String>) = t(key, args)
    // DM の名前は内部の識別子なので、一覧と同じ総称を出す。
    fun label(channel: LinkedChannel) = if (channel.isDm) text("dms.title") else "#${channel.name}"
    return when (info) {
        ChannelLinkInfo.Loading -> ChannelLinkView.Loading(text("channelLink.loading"))
        ChannelLinkInfo.Failed -> ChannelLinkView.Notice(UiIcon.CloudOff, text("channelLink.loadFailed"))
        is ChannelLinkInfo.Unavailable -> ChannelLinkView.Notice(
            UiIcon.VisibilityOff,
            text(if (info.message) "channelLink.messageUnavailable" else "channelLink.channelUnavailable"),
        )
        is ChannelLinkInfo.Channel -> ChannelLinkView.Channel(
            channel = label(info.channel),
            note = when {
                info.channel.encrypted -> text("channelLink.encryptedChannel")
                info.channel.isPrivate && !info.channel.isDm -> text("channelLink.privateChannel")
                else -> null
            },
            description = info.channel.description,
        )
        is ChannelLinkInfo.Message -> {
            val channel = label(info.channel)
            val message = info.message
            ChannelLinkView.Message(
                inlineLabel = text("channelLink.inlineMessage", "name" to message.authorName, "channel" to channel),
                author = message.authorName,
                time = time(message.createdAt),
                channel = if (message.parentId != null) text("channelLink.thread", "channel" to channel) else channel,
                body = channelLinkBody(message, ::text),
                parentId = message.parentId,
            )
        }
    }
}

private fun channelLinkBody(message: LinkedMessage, text: (String, Array<out Pair<String, String>>) -> String) = when {
    message.kind == LinkedMessageKind.Encrypted ->
        ChannelLinkBody(text("channelLink.encrypted", emptyArray()), UiIcon.Lock, italic = true)
    message.kind == LinkedMessageKind.App ->
        ChannelLinkBody(text("channelLink.appMessage", emptyArray()), UiIcon.Widgets)
    message.excerpt.isNotEmpty() -> ChannelLinkBody(message.excerpt)
    // コメントのない転送。転送元は展開せず、通知と同じ文言で、転送したことだけを伝える。
    message.forwarded -> ChannelLinkBody(text("notifications.forwardedPreview", emptyArray()), UiIcon.Forward)
    message.attachmentCount > 0 -> ChannelLinkBody(
        text("channelLink.attachments", arrayOf("count" to message.attachmentCount.toString())), UiIcon.Attach,
    )
    else -> null
}
