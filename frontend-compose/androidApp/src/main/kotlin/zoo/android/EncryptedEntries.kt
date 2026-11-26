package zoo.android

import kotlinx.serialization.json.JsonObject
import zoo.DecryptedBody
import zoo.RecipientNotice
import zoo.Row
import zoo.ENCRYPTED_ATTACHMENT_CONTENT_TYPE
import zoo.attachmentType
import zoo.android.ui.TimelineItem
import zoo.isPlaintextBeforeEncryption
import zoo.text

/**
 * 暗号化チャンネルの 1 行の表示（docs/e2ee.md の「受信時の検証」と #367・#564。Web の `MessageRow` と同じ）。
 *
 * - 復号できなかった行は、本文の代わりに理由を出す（鍵がまだ届いていない、検証に失敗した、など）。
 * - 復号できた行は、送信者の確かさ（未検証の送信者、送信者を確かめられない）と版の警告を印で出す。
 * - 暗号化に切り替える前の平文の行は本文をそのまま出して編集させず、最初の暗号化したメッセージの前に
 *   区切りを出す。それより後ろにある平文の行だけに「暗号化されていません」の印を付ける。
 * - 暗号化した添付は、復号した本文の `attachments` から名前だけを出す（鍵は [TimelineItem.encryptedFiles]）。
 */
internal fun encryptedEntry(
    entry: TimelineItem,
    row: Row,
    decrypted: DecryptedBody?,
    encryptedChannel: Boolean,
    t: (String) -> String,
): TimelineItem {
    if (decrypted == null) {
        if (!encryptedChannel) return entry
        // 暗号化に切り替える前の平文の行（暗号文も epoch もない）だけ、本文を出す。編集はさせない。
        if (isPlaintextBeforeEncryption(row)) return entry.copy(editable = false)
        // epoch があるのに暗号文のない行は、正しいサーバーは作らない。本文を出さない（Web の `withUnencryptedRows`）。
        return entry.copy(body = "", notice = t("e2ee.undecryptable.unencrypted"), editable = false,
            attachments = emptyList(), media = emptyList(), link = null, linkCard = null, channelLinks = emptyList())
    }
    val marks = listOfNotNull(
        decrypted.notice?.takeIf { decrypted.senderWarning }?.let(t),
        decrypted.revisionNotice?.let(t),
    )
    val files = decrypted.attachments.associateBy { it.text("attachment_id") }.filterKeys { it.isNotEmpty() }
    return entry.copy(
        body = decrypted.body.orEmpty(),
        notice = decrypted.notice?.takeIf { decrypted.body == null }?.let(t),
        marks = marks,
        // 復号できない行と、古い版を返された行は編集させない（古い本文から作った版で上書きしないため）。
        editable = entry.editable && decrypted.body != null && decrypted.revisionNotice == null,
        attachments = files.map { (id, info) -> id to receivedAttachment(info).name.ifBlank { id } },
        encryptedFiles = files.mapValues { it.value.toString() },
        media = emptyList(),
        link = null,
        linkCard = null,
    )
}

/**
 * 表示する一覧（会話の画面なら親のない行、スレッドの画面なら親と返信）の中で、暗号化の境目を決める（#367・#564。
 * Web の `MessageList` が表示する `items` の中で決めるのと同じ）。最初の暗号化したメッセージの前に区切りを出し、
 * それより後ろにある、暗号化に切り替える前の形の平文の行だけに「暗号化されていません」の印を付ける。
 */
internal fun withEncryptionBoundary(
    entries: List<TimelineItem>,
    rows: Map<String, Row>,
    encryptedChannel: Boolean,
    t: (String) -> String,
): List<TimelineItem> {
    if (!encryptedChannel) return entries
    val first = entries.indexOfFirst { entry -> rows[entry.id]?.let { !isPlaintextBeforeEncryption(it) } == true }
    if (first < 0) return entries
    return entries.mapIndexed { index, entry ->
        when {
            index == first && index > 0 -> entry.copy(divider = t("e2ee.legacy.divider"))
            index > first && rows[entry.id]?.let(::isPlaintextBeforeEncryption) == true ->
                entry.copy(marks = entry.marks + t("e2ee.legacy.unencryptedBadge"))
            else -> entry
        }
    }
}

/** 送信画面の表示の文（Web の `EncryptionRecipientsNotice`）。何もなければ空。 */
internal fun recipientNoticeLines(
    notice: RecipientNotice?,
    name: (String) -> String,
    t: (String, Array<out Pair<String, String>>) -> String,
): List<String> {
    if (notice == null || notice.isEmpty) return emptyList()
    return buildList {
        if (notice.added.isNotEmpty()) add(t("e2ee.recipients.added", arrayOf("names" to notice.added.joinToString(", ") { name(it) })))
        if (notice.undeliverable.isNotEmpty()) {
            add(t("e2ee.undeliverable.title", emptyArray()))
            notice.undeliverable.forEach { (user, reason) -> add(t(reason.key, arrayOf("name" to name(user)))) }
        }
    }
}

/** 名前の双方向の制御文字（Web の `BIDI_CONTROLS`）。拡張子を偽装できるので `_` に置き換える。 */
private val BIDI_CONTROLS = Regex("[\u200e\u200f\u202a-\u202e\u2066-\u2069]")

/** 受け取った暗号化した添付の、表示と保存に使う名前と MIME（Web の `receivedAttachmentView`）。 */
internal data class ReceivedAttachment(val name: String, val mimetype: String, val blocked: Boolean)

/**
 * 送信者が本文に書いた名前と MIME を、平文の添付と同じ規則（[attachmentType]）で確かめる。平文の添付ならサーバーが
 * 拒否する種類（実行形式など）は `blocked` にし、取得も保存もしない。
 */
internal fun receivedAttachment(info: JsonObject): ReceivedAttachment {
    val name = info.text("name").replace(BIDI_CONTROLS, "_")
    val mimetype = attachmentType(name, info.text("mimetype"))
    return if (mimetype == null) ReceivedAttachment(name, ENCRYPTED_ATTACHMENT_CONTENT_TYPE, blocked = true)
    else ReceivedAttachment(name, mimetype, blocked = false)
}
