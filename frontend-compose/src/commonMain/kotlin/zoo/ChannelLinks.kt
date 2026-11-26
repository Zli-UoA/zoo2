package zoo

import io.ktor.http.Url
import io.ktor.http.encodeURLParameter
import kotlinx.serialization.json.JsonArray

/**
 * このアプリのチャンネル・メッセージへのリンク（docs/mfm.md の「チャンネル・メッセージのリンクの表示」）。
 * `messageId` は `?message=`、`threadId` は `?thread=`。どちらもなければチャンネルそのものを指す。
 */
data class ChannelLinkTarget(val channelId: String, val messageId: String? = null, val threadId: String? = null) {
    /** リンクが指すメッセージ。`?message=` を優先し、`?thread=` だけのリンクはスレッドの親を指す。 */
    val linkedMessageId: String? get() = messageId ?: threadId
}

private val channelLinkPath = Regex("^/channels/([^/]+)/?$")

private val messageIdPattern = Regex("[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}|[0-9a-fA-F]{32}")

/**
 * メッセージの ID（UUID）の形か。ほかの値は、どのメッセージも指さない。大文字などにそろえ直さず、書いてあるまま
 * 照合する（サーバーは、投稿者が指定した ID をそのまま保存する）。
 */
fun isMessageId(value: String): Boolean = messageIdPattern.matches(value)

/**
 * [url] が、接続先（[appBaseUrl]）と同じ origin の `/channels/<ID>` なら行き先を返す（Web の `parseChannelLink`）。
 * 別の origin、別のパス、ユーザー情報の付いた URL は `null`（外部のリンクとして扱う）。
 */
fun parseChannelLink(url: String, appBaseUrl: String): ChannelLinkTarget? {
    val link = publicUrl(url) ?: return null
    val app = publicUrl(appBaseUrl) ?: return null
    if (link.protocol.name != app.protocol.name || !link.host.equals(app.host, ignoreCase = true) ||
        link.port != app.port
    ) return null
    val match = channelLinkPath.find(link.encodedPath) ?: return null
    return try {
        ChannelLinkTarget(
            channelId = match.groupValues[1].decodeURLPart(),
            messageId = link.parameters["message"]?.takeIf { it.isNotEmpty() },
            threadId = link.parameters["thread"]?.takeIf { it.isNotEmpty() },
        )
    } catch (_: Exception) {
        null
    }
}

/**
 * アプリ内で行き先へ移動するパス。`message` はスレッドの返信へ移動する対象で、`thread` は開くスレッド。
 * [open] は、同じ行き先をもう一度押したときにも、行へ移動し直すための値。
 */
fun channelLinkDestination(target: ChannelLinkTarget, thread: String?, open: Long): String = buildString {
    append("/channels/").append(target.channelId.encodeURLParameter())
    val query = buildList {
        target.linkedMessageId?.let { add("message=${it.encodeURLParameter()}") }
        thread?.let { add("thread=${it.encodeURLParameter()}") }
        add("open=$open")
    }
    append('?').append(query.joinToString("&"))
}

/** パスの `?thread=`（アプリ内のリンクで開くスレッド）。なければ `null`。 */
fun linkedThread(path: String): String? =
    if (!path.startsWith("/channels/")) null
    else Url("https://zoo.invalid$path").parameters["thread"]?.takeIf { it.isNotEmpty() }

/** 1 つの本文から取得するリンク先のメッセージの数。他の人が書いた 1 件の投稿で、購読や要求が増え続けないようにする。 */
const val LINK_FETCH_LIMIT = 3

/**
 * 本文のリンクのうち、取得するメッセージ（チャンネル ID とメッセージ ID）。[embedded]（投稿の下の埋め込み）を先に、
 * 続けて [bare]（URL の文字そのもの）を、メッセージごとに数えて [LINK_FETCH_LIMIT] 件まで。チャンネルだけのリンクは
 * 取得しないので数えない。
 */
fun fetchedLinks(embedded: List<ChannelLinkTarget>, bare: List<ChannelLinkTarget>): Set<Pair<String, String>> =
    (embedded + bare)
        .mapNotNull { target -> target.linkedMessageId?.let { target.channelId to it } }
        .distinct()
        .take(LINK_FETCH_LIMIT)
        .toSet()

/** 投稿の下に出すリンク。`channels` はこのアプリのリンク、`external` はそれ以外の最初の URL。 */
data class MessageLinks(val channels: List<Pair<String, ChannelLinkTarget>>, val external: String?)

/**
 * 投稿の下に出すリンクを、このアプリのリンク（埋め込み。Summaly に送らない）と外部の URL に分ける。
 * Web と同じく、合わせて最大 3 件（[previewUrls]）。
 */
fun messageLinks(body: String, appBaseUrl: String, excludeUrl: String? = null): MessageLinks {
    val channels = mutableListOf<Pair<String, ChannelLinkTarget>>()
    var external: String? = null
    for (url in previewUrls(body, excludeUrl)) {
        val target = parseChannelLink(url, appBaseUrl)
        if (target != null) channels += url to target
        else if (external == null) external = url
    }
    return MessageLinks(channels, external)
}

/** 埋め込みカードと短いリンクに使う、リンク先のチャンネル。 */
data class LinkedChannel(
    val id: String,
    val name: String,
    val description: String,
    val isPrivate: Boolean,
    val isDm: Boolean,
    /** サーバーの `encrypted`。表示の判定は [treatsLinkAsEncrypted]。 */
    val encrypted: Boolean,
)

/** 見られるチャンネル（`channels` Shape の行のうち、削除されていないもの）。ないチャンネルは「見られない」。 */
fun linkedChannel(rows: List<Row>, channelId: String): LinkedChannel? =
    rows.firstOrNull { it.text("id") == channelId && !it.present("deleted_at") }?.let {
        LinkedChannel(
            id = channelId,
            name = it.text("channel_name"),
            description = it.text("description"),
            isPrivate = it.flag("is_private"),
            isDm = it.flag("is_dm"),
            encrypted = it.flag("encrypted"),
        )
    }

/**
 * リンク先を暗号化チャンネルとして扱うか（docs/e2ee.md の「暗号化の格下げを防ぐ」）。サーバーが
 * `encrypted = false` を配っても、端末の記録・`edm:` の名前・読み込み済みの暗号文の行があれば、暗号化として扱う。
 */
fun treatsLinkAsEncrypted(channel: LinkedChannel, recorded: Boolean, hasCiphertext: Boolean): Boolean =
    channel.encrypted || recorded || channel.name.startsWith("edm:") || hasCiphertext

enum class LinkedMessageKind {
    /** 本文の要約を出す。 */
    Text,
    /** 暗号化されたメッセージ。本文は出さない。 */
    Encrypted,
    /** 連携カード・部品・App のブロックの投稿。`content` は出さない。 */
    App,
}

data class LinkedMessage(
    val id: String,
    /** 返信なら、そのスレッドの親。 */
    val parentId: String?,
    val authorName: String,
    val createdAt: String,
    val kind: LinkedMessageKind,
    /** 本文の要約。[kind] が [LinkedMessageKind.Text] のときだけ入る。 */
    val excerpt: String,
    val attachmentCount: Int,
    /** 別のメッセージを転送（引用）した投稿か。暗号化されたメッセージは常に false。 */
    val forwarded: Boolean,
)

/** アプリ内リンクの行き先が、いま分かっていること（Web の `ChannelLinkInfo`）。 */
sealed interface ChannelLinkInfo {
    /** チャンネルの一覧かメッセージを取得中。 */
    data object Loading : ChannelLinkInfo

    /** 見られない。存在しない・削除済み・非公開で未参加を区別しない。 */
    data class Unavailable(val message: Boolean) : ChannelLinkInfo

    /** 取得できなかった。見られるかは分からない。 */
    data object Failed : ChannelLinkInfo

    data class Channel(val channel: LinkedChannel) : ChannelLinkInfo

    data class Message(val channel: LinkedChannel, val message: LinkedMessage) : ChannelLinkInfo
}

/** 端末の暗号化チャンネルの記録の読み込み状態。 */
enum class EncryptionRecords { Loading, Loaded, Unreadable }

/**
 * リンク先の状態を決める（Web の `resolveChannelLinkInfo`）。
 *
 * - [channel] は見られるチャンネルの一覧の行。`null` は、一覧の同期（[directoryReady]）が終わるまで読み込み中。
 * - [message] はリンク先のメッセージの取得の状態。チャンネルだけのリンクでは使わない。
 * - [recorded] は端末の暗号化チャンネルの記録にあるか。[records] が読み終わるまで、平文の本文は出さない。
 * - [hidesContent] は、本文の代わりにカード・部品を描く投稿か。
 */
fun resolveChannelLink(
    target: ChannelLinkTarget,
    channel: LinkedChannel?,
    directoryReady: Boolean,
    message: LinkedMessageState,
    recorded: Boolean,
    records: EncryptionRecords,
    users: Map<String, Row>,
    hidesContent: (Row) -> Boolean,
): ChannelLinkInfo {
    val messageId = target.linkedMessageId
    if (channel == null) {
        return if (directoryReady) ChannelLinkInfo.Unavailable(message = messageId != null)
        else ChannelLinkInfo.Loading
    }
    val encrypted = treatsLinkAsEncrypted(channel, recorded, message.hasCiphertext)
    val shown = if (encrypted && !channel.encrypted) channel.copy(encrypted = true) else channel
    if (messageId == null) return ChannelLinkInfo.Channel(shown)
    val row = when (val current = message.row) {
        LinkedRow.Loading -> return if (message.stalled) ChannelLinkInfo.Failed else ChannelLinkInfo.Loading
        LinkedRow.Failed -> return ChannelLinkInfo.Failed
        LinkedRow.Missing -> return ChannelLinkInfo.Unavailable(message = true)
        is LinkedRow.Found -> current.row
    }
    if (row.text("id") != messageId || row.present("deleted_at")) {
        return ChannelLinkInfo.Unavailable(message = true)
    }
    val linked = linkedMessage(row, shown, users, hidesContent)
    // 暗号化かどうかが決まるまで、本文は出さない。本文を出さない種類（暗号化・App）は待たない。
    if (linked.kind == LinkedMessageKind.Text && records != EncryptionRecords.Loaded) {
        return if (records == EncryptionRecords.Unreadable) ChannelLinkInfo.Failed else ChannelLinkInfo.Loading
    }
    return ChannelLinkInfo.Message(shown, linked)
}

internal fun linkedMessage(
    row: Row,
    channel: LinkedChannel,
    users: Map<String, Row>,
    hidesContent: (Row) -> Boolean,
): LinkedMessage {
    // 暗号文のある行は、チャンネルの `encrypted` がどうあっても暗号化されたメッセージとして扱う。
    val kind = when {
        channel.encrypted || row.present("ciphertext") -> LinkedMessageKind.Encrypted
        hidesContent(row) -> LinkedMessageKind.App
        else -> LinkedMessageKind.Text
    }
    val userId = row.text("user_id")
    val authorName = row.text("display_name").ifBlank {
        users[userId]?.let(::author) ?: row.text("user_name").ifBlank { userId }
    }
    val attachments = (row.nested("display_data")["attachments"] as? JsonArray)?.size ?: 0
    return LinkedMessage(
        id = row.text("id"),
        parentId = row.text("parent_id").takeIf { it.isNotEmpty() },
        authorName = authorName,
        createdAt = row.text("created_at"),
        kind = kind,
        excerpt = if (kind == LinkedMessageKind.Text) messageExcerpt(row.text("content")) else "",
        attachmentCount = attachments,
        forwarded = kind != LinkedMessageKind.Encrypted && row.array("forwarded_message_ids").isNotEmpty(),
    )
}
