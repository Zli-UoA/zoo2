package zoo.android

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
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
import zoo.messageExcerptNeedsWorkspaceEmoji
import zoo.parseChannelLink
import zoo.resolveChannelLink
import zoo.text

/** 本文のアプリ内リンクを、行き先の状態を購読して表示する [ChannelLinkProvider]。 */
@Composable
internal fun rememberChannelLinks(
    session: Session, platform: AndroidPlatform, workspaceEmojis: () -> Map<String, String>,
    onEmojiDemand: (Boolean) -> Unit, open: (ChannelLinkTarget, String?) -> Unit,
): ChannelLinkProvider {
    // 一覧の購読と索引は、リンクごとに作らず、画面で 1 つだけ持つ。値を読むのは各リンクの表示だけ。
    val channels = session.repo.channels.rows.collectAsState()
    val synchronized = session.repo.channels.synchronizedRows.collectAsState()
    val users = session.repo.users.rows.collectAsState()
    return remember(session, platform, workspaceEmojis, onEmojiDemand, open, channels, synchronized, users) {
        val lists = ChannelLinkLists(
            channels = derivedStateOf { channels.value.associateBy { it.text("id") } },
            synchronized = derivedStateOf { synchronized.value != null },
            users = derivedStateOf { users.value.associateBy { it.text("id") } },
        )
        ChannelLinkProvider(
            target = { url -> parseChannelLink(url, session.config.apiBaseUrl) },
            view = { target -> channelLinkState(session, platform, lists, target, workspaceEmojis, onEmojiDemand) },
            open = open,
        )
    }
}

private class ChannelLinkLists(
    val channels: State<Map<String, Row>>,
    /** 一覧の初回同期（保存した控えではなく、サーバーから）が終わったか。終わるまで「見られない」と決めない。 */
    val synchronized: State<Boolean>,
    val users: State<Map<String, Row>>,
)

@Composable
private fun channelLinkState(
    session: Session, platform: AndroidPlatform, lists: ChannelLinkLists, target: ChannelLinkTarget,
    workspaceEmojis: () -> Map<String, String>, onEmojiDemand: (Boolean) -> Unit,
): ChannelLinkView {
    val row = lists.channels.value[target.channelId]
    val channel = remember(row, target.channelId) { linkedChannel(listOfNotNull(row), target.channelId) }
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
    var needsWorkspaceEmoji = false
    val info = resolveChannelLink(
        target, channel, directoryReady = lists.synchronized.value, message = message,
        recorded = channel != null && session.isEncryptedChannel(channel.id), records = records,
        users = lists.users.value,
        customEmojis = { linked ->
            val body = linked.text("content")
            val workspace = workspaceEmojis()
            val codes = timelineBodyEmojis(session.config.apiBaseUrl, linked, linked, body,
                isPost = false, workspace = workspace).keys
            if (!session.workspaceEmojiCatalogIsFresh()) {
                val attachedCodes = if (workspace.isEmpty()) codes else
                    timelineBodyEmojis(session.config.apiBaseUrl, linked, linked, body,
                        isPost = false, workspace = emptyMap()).keys
                needsWorkspaceEmoji = messageExcerptNeedsWorkspaceEmoji(body, attachedCodes)
            }
            codes
        },
        hidesContent = { messageRichContent(it).hidesBody },
    )
    if (info is ChannelLinkInfo.Message && needsWorkspaceEmoji) {
        DisposableEffect(onEmojiDemand, target) {
            onEmojiDemand(true)
            onDispose { onEmojiDemand(false) }
        }
    }
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
