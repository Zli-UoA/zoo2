package zoo

import kotlinx.coroutines.flow.StateFlow
import kotlinx.serialization.json.JsonObject
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue

/** Android の暗号ストアを共有チャットから使う。平文を同期キャッシュには書き込まない。 */
interface MessageEncryption {
    val changes: StateFlow<Long>

    /**
     * 最後に配布先を判定したときの、チャンネルごとの送信画面の表示（docs/e2ee.md の「鍵の配布先の判定」。
     * Web の `EncryptionRecipientsNotice`）。判定したことのないチャンネルは入らない。
     */
    val recipients: StateFlow<Map<String, RecipientNotice>>

    /** 暗号化した添付の暗号の処理（#364）。 */
    val attachmentCipher: AttachmentCipher

    /**
     * `attachments` は本文の `attachments` に入れる値（[uploadEncryptedAttachment] の戻り値）。鍵を含むので、
     * 暗号化した本文にだけ入れ、送信の要求には `attachment_ids` だけを入れる。
     */
    suspend fun send(
        channel: String,
        id: String,
        parent: String?,
        body: String,
        mentions: EncryptedMentions = EncryptedMentions(),
        attachments: List<JsonObject> = emptyList(),
    )

    suspend fun edit(message: Row, body: String)
    suspend fun decrypt(message: Row): DecryptedBody
}

/** 鍵を配れない理由（Web の `UndeliverableReason`）。`key` は翻訳のキー。 */
enum class UndeliverableReason(val key: String) {
    NOT_SET_UP("e2ee.undeliverable.notSetUp"),
    INVALID_KEYS("e2ee.undeliverable.invalidKeys"),
    NEEDS_REVERIFICATION("e2ee.undeliverable.needsReverification"),
    PIN_PENDING("e2ee.undeliverable.pinPending"),
    BLOCKED("e2ee.undeliverable.blocked"),
}

/**
 * 送信画面の表示。配布先の数は出さず（毎回同じで読み流されるため）、前回の判定から増えた配布先と、
 * 鍵を配れない相手だけを持つ（#565）。
 */
data class RecipientNotice(
    /** 前回判定した配布先から増えたユーザー（`user_id` の昇順）。初めて判定したチャンネルでは空。 */
    val added: List<String> = emptyList(),
    /** 鍵を配れないユーザーと理由。配れなくても送信は止めない。 */
    val undeliverable: List<Pair<String, UndeliverableReason>> = emptyList(),
) {
    val isEmpty: Boolean get() = added.isEmpty() && undeliverable.isEmpty()
}

enum class MessageRevision { LATEST, STALE, CONFLICT }

data class DecryptedBody(
    val body: String? = null,
    val notice: String? = null,
    val revision: MessageRevision = MessageRevision.LATEST,
    /** 本文の `attachments`（鍵を含む。表示と取得にだけ使い、保存しない）。 */
    val attachments: List<JsonObject> = emptyList(),
    /** 復号した `mentions`。本文の `@name` のうち、ここにあるものだけをメンションとして扱う。 */
    val mentions: EncryptedMentions = EncryptedMentions(),
) {
    val revisionNotice: String? get() = when (revision) {
        MessageRevision.LATEST -> null
        MessageRevision.STALE -> "e2ee.revision.stale"
        MessageRevision.CONFLICT -> "e2ee.revision.conflict"
    }

    /** 送信者の確かさの印（本文は出す）。復号できなかった理由（本文を出さない）とは別。 */
    val senderWarning: Boolean get() = body != null && notice != null
}

/** 表示メタデータを復号処理のキーに含めない。 */
internal fun encryptedMessagePayload(message: Row): Row = kotlinx.serialization.json.JsonObject(
    message.filterKeys { it in setOf("channel_id", "id", "parent_id", "user_id", "kind", "epoch", "revision", "ciphertext") }
)

/**
 * 暗号化チャンネルで、暗号化に切り替える前の平文の行か（Web の `isPlaintextBeforeEncryption`）。
 * 暗号文も epoch もない行。
 */
fun isPlaintextBeforeEncryption(message: Row): Boolean =
    !message.present("ciphertext") && !message.present("epoch")

/**
 * 並べた行のうち、最初の暗号化したメッセージの位置（#367・#564）。ここに「ここから暗号化されています」の
 * 区切りを出し、これより後ろの暗号文のない行には警告の印を付ける。暗号化チャンネルでなければ -1。
 */
fun firstEncryptedIndex(rows: List<Row>, encrypted: Boolean): Int =
    if (!encrypted) -1 else rows.indexOfFirst { !isPlaintextBeforeEncryption(it) }

/** 確認済みの本文は画面内だけで保持する。旧版・同版の競合で置き換えない。 */
internal class EncryptedMessageDisplayState {
    var decoded by androidx.compose.runtime.mutableStateOf<DecryptedBody?>(null)
        private set

    fun receive(result: DecryptedBody) {
        decoded = if (result.revision == MessageRevision.LATEST) result
        else result.copy(body = decoded?.body, notice = decoded?.notice,
            attachments = decoded?.attachments.orEmpty(), mentions = decoded?.mentions ?: EncryptedMentions())
    }
}
