package zoo.android

import kotlinx.serialization.json.JsonObject
import zoo.DecryptedBody
import zoo.RecipientNotice
import zoo.Row
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
    index: Int,
    firstEncrypted: Int,
    t: (String) -> String,
): TimelineItem {
    val divider = if (index > 0 && index == firstEncrypted) t("e2ee.legacy.divider") else null
    if (decrypted == null) {
        if (!encryptedChannel || !isPlaintextBeforeEncryption(row)) return entry.copy(divider = divider)
        val marks = if (firstEncrypted in 0 until index) listOf(t("e2ee.legacy.unencryptedBadge")) else emptyList()
        return entry.copy(editable = false, marks = marks, divider = divider)
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
        divider = divider,
        // 復号できない行と、古い版を返された行は編集させない（古い本文から作った版で上書きしないため）。
        editable = entry.editable && decrypted.body != null && decrypted.revisionNotice == null,
        attachments = files.map { (id, info) -> id to info.text("name").ifBlank { id } },
        encryptedFiles = files,
        media = emptyList(),
        link = null,
        linkCard = null,
    )
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

/** 暗号化した添付を開くときの MIME。ブラウザーなどで実行されうる種類は開かずに保存させる（Web の `blocked`）。 */
internal fun safeAttachmentType(info: JsonObject): String {
    val type = info.text("mimetype").lowercase()
    val active = type.isBlank() || type.contains("html") || type.contains("svg") || type.contains("xml") ||
        type.contains("javascript")
    return if (active) "application/octet-stream" else type
}
