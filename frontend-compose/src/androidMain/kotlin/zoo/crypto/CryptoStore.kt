package zoo.crypto

import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import zoo.crypto.ffi.*
import zoo.crypto.ffi.AddOutcome
import zoo.crypto.ffi.BackupUpload
import zoo.crypto.ffi.BackupUploadState
import zoo.crypto.ffi.DeviceAccount
import zoo.crypto.ffi.InboundSessionStore
import zoo.crypto.ffi.OlmSessionStore
import zoo.crypto.ffi.OwnKeys
import zoo.crypto.ffi.SenderTrust
import zoo.crypto.ffi.SessionRef
import zoo.crypto.ffi.SetupSession
import zoo.crypto.ffi.StartupCheck
import zoo.crypto.ffi.TrustStore
import zoo.crypto.ffi.VersionChange
import zoo.crypto.ffi.ZooCryptoException

/**
 * 暗号ストアの置き場所（docs/e2ee.md の「暗号ストア」、#359）。
 *
 * Android では、接続先とユーザーごとに、アプリの内部ストレージのうち自動バックアップと端末間の
 * 転送に入らない `noBackupFilesDir/e2ee/<ID>/` に置く（[android]）。保存鍵を包む Keystore の鍵も
 * 暗号ストアごとに分け、ログアウトでは包む鍵から消す。
 */
class CryptoStoreFiles(
    /** `state.json`（暗号ストアの状態）と `storage.key`（包んだ保存鍵）を置くディレクトリー。 */
    val directory: File,
    /** 保存鍵を包む鍵。 */
    val wrappingKey: WrappingKeyProvider,
    /** 包む鍵を消す。消すと、包んだ保存鍵はもう開けない。 */
    val deleteWrappingKey: () -> Unit,
) {
    companion object {
        /** `noBackupFilesDir/e2ee`。削除待ちの一覧（[PendingDeviceDeletions]）もここに置く。 */
        fun root(context: android.content.Context): File = File(context.noBackupFilesDir, "e2ee")

        /** 接続先 `server` のユーザー `userId` の暗号ストア。 */
        fun android(context: android.content.Context, server: String, userId: String): CryptoStoreFiles {
            val id = storeId(server, userId)
            val key = AndroidKeystoreWrappingKey("${AndroidKeystoreWrappingKey.DEFAULT_ALIAS}:$id")
            return CryptoStoreFiles(File(root(context), id), key, key::delete)
        }

        /** 接続先とユーザーから作る、ディレクトリー名と Keystore の別名に使う ID。 */
        fun storeId(server: String, userId: String): String =
            MessageDigest.getInstance("SHA-256")
                .digest("$server\n$userId".encodeToByteArray())
                .joinToString("") { "%02x".format(it) }
                .take(32)
    }
}

/** 同じ暗号ストアを、ほかの場所（前の画面のセッションなど）が開いている。少し待って試し直す。 */
class CryptoStoreBusyException : IOException("暗号ストアをほかの処理が使っています")

/** to-device の Shape の行のうち、処理に使う列。 */
data class ToDeviceRow(
    val id: String,
    val senderUserId: String,
    val type: String,
    val content: JsonElement,
)

/** [CryptoStore.processToDevice] の結果。 */
data class ToDeviceResult(
    /** 処理を終えた（保存した、または捨てた）ID。ack を送ってよい。 */
    val processed: List<String>,
    val roomKeys: Int,
    val dropped: Int,
    /** 新しく作った Olm セッションの数（ワンタイム鍵を使われたので補充する）。 */
    val newOlmSessions: Int,
    /** 保存した鍵のうち、送信端末を検証できた（受信時の検証の手順 4）ものの数。 */
    val verifiedRoomKeys: Int = 0,
    /**
     * 受信時の検証を通った、他のユーザーの検証のメッセージ（`m.key.verification.*`。#368）。Olm のメッセージは
     * 2 回は復号できないので、ここで返したものを [SasController.handleMessages] に渡して進める。
     */
    val verifications: List<VerificationMessage> = emptyList(),
)

/** 他のユーザーの検証のメッセージの `type` の接頭辞（Web の `VERIFICATION_TYPE_PREFIX`）。 */
internal const val VERIFICATION_TYPE_PREFIX = "m.key.verification."

/** `POST /api/e2ee/keys/upload` に送る鍵。 */
class KeyUpload(val oneTimeKeys: JsonObject, val fallbackKeys: JsonObject)

@Serializable
internal class StoredOlmSession(
    @SerialName("their_identity_key") val theirIdentityKey: String,
    @SerialName("session_id") val sessionId: String,
    val pickle: String,
)

@Serializable
internal class StoredInboundSession(
    val pickle: String,
    /** 鍵バックアップに送ったか。送っていないものの数を、ログアウトの前に見せる。 */
    @SerialName("backed_up") val backedUp: Boolean = false,
)

/**
 * `state.json` の中身。値はどれも保存鍵で暗号化してある（`account` は `DeviceAccount.pickle`、
 * ほかは zoo-crypto の保存用の封筒）。`user_id`、`device_id`、登録済みか、処理済みの to-device の ID
 * だけが平文で、秘密ではない（TUI の `state.json` と同じ形）。
 */
@Serializable
internal class StoreState(
    val version: Int,
    @SerialName("user_id") val userId: String,
    @SerialName("device_id") val deviceId: String,
    var registered: Boolean = false,
    var account: String,
    @SerialName("olm_sessions") var olmSessions: List<StoredOlmSession> = emptyList(),
    @SerialName("inbound_sessions") var inboundSessions: Map<String, StoredInboundSession> = emptyMap(),
    @SerialName("trust_store") var trustStore: String? = null,
    /** 処理を終えて、まだ ack を送れていない to-device の ID。 */
    @SerialName("processed_to_device") var processedToDevice: Set<String> = emptySet(),
    /** 解錠した（または初回設定で作った）自分の鍵（`OwnKeys` の封筒。マスター鍵の秘密鍵は入れない。#360）。 */
    @SerialName("own_keys") var ownKeys: String? = null,
    /** 最後に解錠したとき（自分で SSSS を書いたときは、その応答）の `secrets_revision`。 */
    @SerialName("secrets_revision") var secretsRevision: Long? = null,
    /** 鍵バックアップの送信の状態（`BackupUploadState.to_json`。秘密は入らない）。 */
    @SerialName("backup_upload") var backupUpload: String? = null,
    /** 解錠したあと、過去のセッションを最後まで取り込んだ鍵バックアップの版（新しい端末での解錠の手順 5）。 */
    @SerialName("imported_backup_version") var importedBackupVersion: String? = null,
    /**
     * チャンネルごとに、前回判定した配布先のメンバー（`user_id` の昇順。Web の `meta` の `recipients:`、TUI の
     * `recipients` と同じ役割）。送信画面に「前回の送信から増えた配布先」を出すのに使う。秘密ではない。
     */
    var recipients: Map<String, List<String>> = emptyMap(),
    /**
     * 終わっていない失効の進み具合（#362。`Revocation.pickle` の封筒。保存鍵で暗号化してある）。新しい自己署名鍵、
     * ユーザー署名鍵、バックアップ鍵と古いバックアップ鍵を含み、マスター鍵の秘密鍵と SSSS 鍵は含まない（Web の
     * `meta` の `revocation` と同じ）。端末を作り直すとき（[CryptoStore.replaceDevice]）は捨てる。
     */
    var revocation: String? = null,
)

/** [CryptoStore.prepareBackupUpload] の結果。 */
sealed interface BackupUploadRequest {
    /** 送るものがない（解錠していない、今の版がない、送信待ちがない）。 */
    data object None : BackupUploadRequest

    /** 今の版を使えない（署名がない、SSSS の鍵と違う、版の巻き戻りなど）。送らない。 */
    data class Rejected(val reason: String) : BackupUploadRequest

    /** `POST /api/e2ee/key_backup/keys` に送る。 */
    class Upload(
        val id: Long,
        val version: String,
        val secretsRevision: Long,
        val body: JsonObject,
        val count: Int,
        /**
         * `X-Zoo-Backup-Proof` の値（docs/e2ee.md の「アップロードの証明」）。今の版に `upload_key` がない
         * （証明を足す前のサーバー）ときだけ `null` で、ヘッダーを送らない。
         */
        val proof: String? = null,
    ) : BackupUploadRequest
}

/**
 * Android の暗号ストア（docs/e2ee.md の「暗号ストア」、#359）。1 つのユーザーの端末アカウント、
 * Olm セッション、受信用セッション、配布先の判定の記録を持つ。
 *
 * - `state.json` は一時ファイルに書いて `fsync` してから置き換えるので、書いている途中で終了しても、
 *   前の状態か次の状態のどちらかが残る。
 * - 保存鍵（32 バイト）は [StorageKeyStore] で Android Keystore の鍵で包み、`storage.key` に置く。
 * - 同じ暗号ストアを同時に 2 つ開かない（Olm のメッセージは 2 回は復号できないため）。開いているあいだは
 *   [CryptoStoreBusyException] で断る。
 *
 * どのメソッドも、保存（`state.json` の書き込み）に失敗したら例外を投げる。そのときはメモリーの状態が
 * 保存より先に進んでいるので、呼び出し側は閉じて開き直してから続ける（[E2eeDeviceManager]）。
 */
class CryptoStore private constructor(
    private val directory: File,
    private val storageKey: ByteArray,
    private val account: DeviceAccount,
    private val olmSessions: OlmSessionStore,
    private val inbound: InboundSessionStore,
    private var trust: TrustStore,
    private val state: StoreState,
    private val deleteWrappingKey: () -> Unit,
) : AutoCloseable {
    val changes = kotlinx.coroutines.flow.MutableStateFlow(0L)
    private val outboundOlm = mutableMapOf<String, String>()
    private val loadedInbound = mutableSetOf<String>()
    private var closed = false

    /** 鍵バックアップの送信の状態。初めて送るときに `state.backup_upload` から読む。 */
    private var backupState: BackupUploadState? = null

    /** 送っている途中の要求と、送ったときのセッションの pickle（送ったあとに書き換わったものを残すため）。 */
    private class PendingUpload(val upload: BackupUpload, val sessions: List<Pair<String, String>>)

    private val pendingUploads = mutableMapOf<Long, PendingUpload>()
    private var nextUploadId = 0L

    val userId: String get() = state.userId
    val deviceId: String get() = state.deviceId
    @get:Synchronized
    val isRegistered: Boolean get() = state.registered

    /** 開いた結果。保存鍵を失って作り直したら、前の端末の ID を返す（サーバーから消す）。 */
    class Opened(val store: CryptoStore, val replacedDeviceId: String?)

    /** `POST /api/e2ee/devices` の本文（`display_name` を除く）。応答をなくしても同じ鍵で送り直す。 */
    @Synchronized
    fun registrationRequest(): JsonObject = buildJsonObject {
        put("device_keys", json.parseToJsonElement(account.deviceKeys()))
        put("one_time_keys", json.parseToJsonElement(account.oneTimeKeys()))
        put("fallback_keys", json.parseToJsonElement(account.fallbackKeys()))
    }

    @Synchronized
    fun markRegistered() {
        account.markKeysAsPublished()
        state.account = account.pickle(storageKey)
        state.registered = true
        save()
    }

    /**
     * サーバーのワンタイム鍵を最大数に戻し、使われたフォールバック鍵を作り直す。送るものがなければ
     * `null`。まだ公開していない鍵（応答をなくした送信の分）も入れる。
     */
    @Synchronized
    fun prepareKeyUpload(oneTimeKeyCount: Int, hasUnusedFallbackKey: Boolean): KeyUpload? {
        val unpublished = json.parseToJsonElement(account.oneTimeKeys()) as JsonObject
        val max = account.maxNumberOfOneTimeKeys().toLong()
        var changed = false
        val missing = max - oneTimeKeyCount - unpublished.size
        if (missing > 0) {
            account.generateOneTimeKeys(missing.toULong())
            changed = true
        }
        if (!hasUnusedFallbackKey && (json.parseToJsonElement(account.fallbackKeys()) as JsonObject).isEmpty()) {
            account.generateFallbackKey()
            changed = true
        }
        if (changed) {
            state.account = account.pickle(storageKey)
            save()
        }
        val oneTimeKeys = json.parseToJsonElement(account.oneTimeKeys()) as JsonObject
        val fallbackKeys = json.parseToJsonElement(account.fallbackKeys()) as JsonObject
        if (oneTimeKeys.isEmpty() && fallbackKeys.isEmpty()) return null
        return KeyUpload(oneTimeKeys, fallbackKeys)
    }

    @Synchronized
    fun markKeysPublished() {
        account.markKeysAsPublished()
        state.account = account.pickle(storageKey)
        save()
    }

    /**
     * to-device メッセージを処理する（React の `CryptoStore.processToDevice`、TUI と同じ規則）。
     *
     * 1 件ごとに、変わった状態と処理済みの印を保存してから次に進む。ack の前に終了しても、同じ
     * メッセージを 2 回復号しない。`deviceKeys` は送信ユーザーの `keys/query` の応答の `device_keys`、
     * `signatures` は同じ応答の `signatures`（`e2ee_key_signatures` の行）。
     *
     * `keysQuery` があれば、自分と送信者のクロス署名と保存済みの信頼記録で送信端末を検証する。
     * 旧呼び出し元の互換経路では、自分の別の端末だけを検証する。
     */
    @Synchronized
    fun processToDevice(
        rows: List<ToDeviceRow>,
        deviceKeys: JsonObject,
        signatures: JsonArray = JsonArray(emptyList()),
        keysQuery: JsonObject? = null,
    ): ToDeviceResult {
        val processed = mutableListOf<String>()
        var roomKeys = 0
        var dropped = 0
        var newOlmSessions = 0
        var verifiedRoomKeys = 0
        val verifications = mutableListOf<VerificationMessage>()
        // 解錠し直しが要る（secrets_revision を消した）ときは、暗号ストアの自分の鍵を使わない。
        val ownSelfSigning by lazy {
            state.ownKeys?.takeIf { state.secretsRevision != null }?.let { sealed ->
                try {
                    OwnKeys.fromPickle(sealed, account, storageKey).use { it.selfSigningPublicKey() }
                } catch (_: ZooCryptoException) {
                    null
                }
            }
        }
        val trustFor = { sender: String?, device: String? ->
            val selfSigning = if (sender == state.userId && device != null) ownSelfSigning else null
            if (keysQuery != null && sender != null && device != null && state.secretsRevision != null) {
                val result = trust.senderTrust(keysQuery.toString(), sender, device, ownSelfSigning)
                // Olm の更新・受信用鍵・処理済み ID と同じ行単位の保存に含める。
                state.trustStore = trust.pickle(account, storageKey)
                result
            } else selfSigning?.let { key ->
                val signature =
                    signatures.firstNotNullOfOrNull { element ->
                        val row = element as? JsonObject ?: return@firstNotNullOfOrNull null
                        fun text(name: String) = (row[name] as? JsonPrimitive)?.content
                        row["signature"]?.let { it as? JsonPrimitive }?.content?.takeIf {
                            text("signer_user_id") == state.userId &&
                                text("signer_key_id") == "ed25519:$key" &&
                                text("target_user_id") == state.userId &&
                                text("target_key_id") == device
                        }
                    }
                SenderTrust(key, signature)
            }
        }
        // 前の処理で印を付けた行（ack の前に失敗した）。その保存の同期が済んだとは限らない（rename の
        // あとで同期に失敗しても、開き直すと新しい state.json を読めることがある）ので、返す前に
        // 保存し直して、同期を確かめてから ack させる。
        var unsaved = false
        for (row in rows) {
            if (row.id in state.processedToDevice) {
                unsaved = true
            } else {
                if (row.type == "m.room.encrypted") {
                    when (val outcome = processEncrypted(row, deviceKeys, trustFor)) {
                        is Encrypted.Dropped -> dropped++
                        is Encrypted.Decrypted -> {
                            newOlmSessions += outcome.newOlmSessions
                            if (outcome.roomKey) roomKeys++ else if (outcome.rejected) dropped++
                            if (outcome.roomKey && outcome.verified) verifiedRoomKeys++
                            outcome.verification?.let(verifications::add)
                        }
                    }
                } else {
                    dropped++
                }
                state.processedToDevice = state.processedToDevice + row.id
                save()
                unsaved = false
            }
            processed += row.id
        }
        if (unsaved) save()
        return ToDeviceResult(processed, roomKeys, dropped, newOlmSessions, verifiedRoomKeys, verifications)
    }

    private sealed interface Encrypted {
        data object Dropped : Encrypted
        data class Decrypted(
            val newOlmSessions: Int,
            val roomKey: Boolean,
            val rejected: Boolean,
            val verified: Boolean = false,
            val verification: VerificationMessage? = null,
        ) : Encrypted
    }

    private fun processEncrypted(
        row: ToDeviceRow,
        deviceKeys: JsonObject,
        trustFor: (sender: String?, device: String?) -> SenderTrust?,
    ): Encrypted {
        val before = olmSessions.len()
        val decrypted =
            try {
                account.decryptToDevice(olmSessions, row.content.toString())
            } catch (_: ZooCryptoException) {
                // 復号できないメッセージは何度試しても読めない。捨てて ack する。
                return Encrypted.Dropped
            }
        decrypted.use {
            // 復号できたら、セッションの状態が進んでいる。検証の結果に関わらず保存する。
            val created = (olmSessions.len() - before).toInt()
            state.account = account.pickle(storageKey)
            val senderKey = decrypted.senderKey()
            val changed = olmSessions.pickleSessions(account, storageKey, senderKey)
            state.olmSessions =
                state.olmSessions.filterNot { stored ->
                    changed.any { it.sessionId == stored.sessionId && it.theirIdentityKey == stored.theirIdentityKey }
                } + changed.map { StoredOlmSession(it.theirIdentityKey, it.sessionId, it.pickle) }
            val listed =
                decrypted.sender()?.let { user ->
                    decrypted.senderDevice()?.let { device -> (deviceKeys[user] as? JsonObject)?.get(device) }
                }
            if (decrypted.eventType()?.startsWith(VERIFICATION_TYPE_PREFIX) == true) {
                // 他のユーザーの検証（#368）。受信時の検証を通したものを返し、検証を進めるのは
                // SasController に任せる（Web の `processEncrypted` と同じ）。
                val verification =
                    try {
                        decrypted.verifyVerification(account, listed?.toString())
                    } catch (_: ZooCryptoException) {
                        return Encrypted.Decrypted(created, roomKey = false, rejected = true)
                    }
                return Encrypted.Decrypted(created, roomKey = false, rejected = false, verification = verification)
            }
            val roomKey =
                try {
                    decrypted.verifyRoomKey(account, listed?.toString(), trustFor(decrypted.sender(), decrypted.senderDevice()))
                } catch (_: ZooCryptoException) {
                    return Encrypted.Decrypted(created, roomKey = false, rejected = true)
                }
            roomKey.use {
                // addRoomKey で鍵を渡す前に読む。
                val verified = roomKey.senderVerified()
                val ref = SessionRef(roomKey.channelId(), roomKey.senderKey(), roomKey.sessionId())
                val key = inboundKey(ref)
                // 読み込めたときだけ読み込み済みにする（失敗したら次の試行で読み直す）。
                loadInbound(key)
                if (inbound.addRoomKey(roomKey) == AddOutcome.KEPT) {
                    return Encrypted.Decrypted(created, roomKey = false, rejected = false)
                }
                val pickle =
                    inbound.pickleSession(account, ref, storageKey)
                        ?: return Encrypted.Decrypted(created, roomKey = false, rejected = false)
                state.inboundSessions = state.inboundSessions + (key to StoredInboundSession(pickle, backedUp = false))
                return Encrypted.Decrypted(created, roomKey = true, rejected = false, verified = verified)
            }
        }
    }

    /** 解錠済みの鍵で配布先を判定し、TOFU の変更を保存する。 */
    @Synchronized
    fun recipients(pages: List<JsonObject>): Distribution {
        check(!closed && state.secretsRevision != null) { "e2ee.sendNeedsUnlock" }
        return OwnKeys.fromPickle(checkNotNull(state.ownKeys), account, storageKey).use { own ->
            // 起動時の確認と送信が重なっても、失効前の自己署名鍵で送信を始めない。
            val serverOwn = pages.firstNotNullOfOrNull { page ->
                ((page["self_signing_keys"] as? JsonObject)?.get(userId) as? JsonObject)
            }
            val signingKeys = serverOwn?.get("keys") as? JsonObject
            check(signingKeys?.values?.any { (it as? JsonPrimitive)?.content == own.selfSigningPublicKey() } == true) {
                "e2ee.sendNeedsUnlock"
            }
            own.userSigning().use { signing ->
                trust.evaluate(pages.map { it.toString() }, deviceId, signing).also { saveTrustStore() }
            }
        }
    }

    /**
     * 配布先のメンバー `userIds` を記録し、前回判定したときから増えたメンバーを返す（Web の
     * `MegolmChannels.noteRecipients`）。初めて判定したチャンネルでは空。
     */
    @Synchronized
    fun noteRecipients(channel: String, userIds: List<String>): List<String> {
        check(!closed)
        val next = userIds.distinct().sorted()
        val previous = state.recipients[channel]
        if (previous != next) {
            val before = state.recipients
            state.recipients = state.recipients + (channel to next)
            try {
                save()
            } catch (failure: Throwable) {
                // 保存できなかったら記録を戻す（次の判定でも、増えた配布先を出すため）。
                state.recipients = before
                throw failure
            }
        }
        if (previous == null) return emptyList()
        val known = previous.toSet()
        return next.filter { it !in known }
    }

    @Synchronized
    fun confirmPin(pin: NewTofuPin, accepted: Boolean) {
        if (accepted) trust.confirmTofuPin(pin) else trust.discardTofuPin(pin)
        saveTrustStore()
    }

    @Synchronized
    fun needsOlm(device: VerifiedDeviceKeys): Boolean = device.curve25519Key() !in outboundOlm

    /**
     * 他のユーザーの検証を求める（#368。Web の `SasVerifications.request`）。`keysQuery` は相手の `keys/query` の
     * 応答（JSON）。解錠していなければ [SasException]（[SasException.NEEDS_UNLOCK]）。
     */
    @Synchronized
    fun requestVerification(keysQuery: String, theirUserId: String, nowMs: Long): SasRequest {
        requireVerificationUnlocked()
        return zoo.crypto.ffi.requestVerification(account, trust, keysQuery, theirUserId, nowMs.toULong())
    }

    /**
     * 届いた要求（`m.key.verification.request`）から検証を作る。古い要求、たどれない端末からの要求、解錠の前
     * （自分のマスター鍵を信頼していない）などは [ZooCryptoException]（捨てる）。
     */
    @Synchronized
    fun verificationFromRequest(message: VerificationMessage, keysQuery: String, nowMs: Long): SasVerification {
        check(!closed) { "暗号ストアは閉じています" }
        return SasVerification.fromRequest(account, trust, message, keysQuery, nowMs.toULong())
    }

    /**
     * 検証できた相手のマスター鍵（`masterKey` は署名の直前に `keys/query` で読んだ JSON）に、自分のユーザー署名鍵で
     * 署名する。記録は登録待ちで保存し、`replace: true` のピンと署名を返す（サーバーに保存できたら
     * [confirmAcceptedMasterKey]）。
     */
    @Synchronized
    fun signVerifiedMasterKey(verification: SasVerification, masterKey: String): AcceptedMasterKey {
        requireVerificationUnlocked()
        return OwnKeys.fromPickle(checkNotNull(state.ownKeys), account, storageKey).use { own ->
            own.userSigning().use { signing -> verification.signMasterKey(trust, signing, masterKey) }
        }.also { saveTrustStore() }
    }

    /** [signVerifiedMasterKey] のピンと署名をサーバーに保存できた。記録の登録待ちを外す。 */
    @Synchronized
    fun confirmAcceptedMasterKey(targetUserId: String, masterKey: String) {
        check(!closed) { "暗号ストアは閉じています" }
        trust.confirmAcceptedMasterKey(targetUserId, masterKey)
        saveTrustStore()
    }

    /**
     * 検証のメッセージを `device` あての to-device の `content` に暗号化する。送信用の Olm セッションは鍵の配布
     * （[shareKey]）と同じものを使い回し、なければ `claimed`（`keys/claim` で払い出した鍵）で作る。どちらもなければ
     * `null`。Olm の状態を進めたので、送る前に永続化する。取りやめも送れるよう、解錠は求めない。
     */
    @Synchronized
    fun encryptVerification(device: VerifiedDeviceKeys, eventType: String, content: String, claimed: String?): JsonElement? {
        check(!closed) { "暗号ストアは閉じています" }
        val curve = device.curve25519Key()
        val session = outboundOlm[curve] ?: claimed?.let {
            account.createOutboundOlmSession(olmSessions, device, it).also { id -> outboundOlm[curve] = id }
        } ?: return null
        val encrypted = account.encryptVerification(olmSessions, session, device, eventType, content)
        persistOutboundOlm(curve)
        return json.parseToJsonElement(encrypted)
    }

    private fun requireVerificationUnlocked() {
        if (closed || state.ownKeys == null || state.secretsRevision == null) {
            throw SasException(SasException.NEEDS_UNLOCK, "解錠していません")
        }
    }

    private fun persistOutboundOlm(curve: String) {
        val changed = olmSessions.pickleSessions(account, storageKey, curve)
        state.olmSessions = state.olmSessions.filterNot { it.theirIdentityKey == curve } +
            changed.map { StoredOlmSession(it.theirIdentityKey, it.sessionId, it.pickle) }
        state.account = account.pickle(storageKey)
        save()
    }

    /** Olm の状態を進めたら、ネットワークへ送る前に永続化する。 */
    @Synchronized
    fun shareKey(device: VerifiedDeviceKeys, outbound: OutboundSession, claimed: String?): JsonElement? {
        check(!closed && state.secretsRevision != null) { "e2ee.sendNeedsUnlock" }
        val curve = device.curve25519Key()
        val session = outboundOlm[curve] ?: claimed?.let {
            account.createOutboundOlmSession(olmSessions, device, it).also { id -> outboundOlm[curve] = id }
        } ?: return null
        val encrypted = account.encryptRoomKey(olmSessions, session, device, outbound)
        persistOutboundOlm(curve)
        return json.parseToJsonElement(encrypted)
    }

    @Synchronized
    fun encrypt(outbound: OutboundSession, message: OutgoingMessage): JsonElement {
        check(!closed && state.secretsRevision != null) { "e2ee.sendNeedsUnlock" }
        // 自分が送った本文も、再起動後に復号・バックアップできるよう受信用の鍵を先に保存する。
        val ref = SessionRef(outbound.channelId(), account.curve25519Key(), outbound.sessionId())
        val key = inboundKey(ref)
        if (key !in state.inboundSessions) {
            loadInbound(key)
            inbound.addOutbound(account, outbound)
            state.inboundSessions = state.inboundSessions +
                (key to StoredInboundSession(checkNotNull(inbound.pickleSession(account, ref, storageKey))))
            save()
        }
        return json.parseToJsonElement(outbound.encrypt(account, message))
    }

    /** channel/id/parent/sender/epoch/revision とリプレイの照合は共通の暗号基盤で行う。 */
    @Synchronized
    fun decrypt(message: EncryptedMessage): DecryptedMessage {
        check(!closed)
        val envelope = json.parseToJsonElement(message.ciphertext) as JsonObject
        fun text(name: String) = (envelope[name] as JsonPrimitive).content
        loadInbound(inboundKey(SessionRef(message.channelId, text("sender_key"), text("session_id"))))
        return inbound.decrypt(message)
    }

    /** ack を送れた ID を、処理済みの印から外す。 */
    @Synchronized
    fun acknowledged(ids: List<String>) {
        if (ids.isEmpty()) return
        state.processedToDevice = state.processedToDevice - ids.toSet()
        save()
    }

    /** 鍵バックアップにまだ送っていない受信用セッションの数。 */
    @Synchronized
    fun pendingBackupCount(): Int = state.inboundSessions.values.count { !it.backedUp }

    /** 保存している受信用セッションのキーの数（単体テストで使う）。 */
    @get:Synchronized
    internal val inboundSessionCount: Int get() = state.inboundSessions.size

    /** 今の端末の署名付きの端末鍵（JSON）。 */
    @Synchronized
    fun deviceKeys(): String = account.deviceKeys()

    /** 自分の鍵を保存してあるか（解錠済みの端末か）。 */
    @get:Synchronized
    val hasOwnKeys: Boolean get() = state.ownKeys != null

    /** 最後に解錠したときの `secrets_revision`。 */
    @get:Synchronized
    val secretsRevision: Long? get() = state.secretsRevision

    /**
     * 解錠した（または初回設定で作った）自分の鍵を保存する（#360。TUI の `save_own_keys`、React の
     * `saveOwnKeys` と同じ）。マスター鍵の秘密鍵は保存せず、自分のマスター鍵の公開鍵とユーザー署名鍵を、
     * 配布先の判定の記録に入れる。書けなければ、メモリーの状態も前に戻して例外を投げる。
     */
    @Synchronized
    fun saveOwnKeys(session: SetupSession, secretsRevision: Long?) {
        // 記録はコピーで更新し、すべて保存できたときだけ置き換える。
        val updated = TrustStore.fromJson(trust.toJson(), state.userId)
        try {
            session.recordOwnKeys(updated)
            val ownKeys = session.ownKeysPickle(account, storageKey)
            val trustStore = updated.pickle(account, storageKey)
            val previous = Triple(state.ownKeys, state.secretsRevision, state.trustStore)
            val previousImported = state.importedBackupVersion
            state.ownKeys = ownKeys
            state.secretsRevision = secretsRevision
            state.trustStore = trustStore
            // 解錠し直したら、鍵バックアップから取り込み直す。
            state.importedBackupVersion = null
            try {
                save()
            } catch (failure: Throwable) {
                state.ownKeys = previous.first
                state.secretsRevision = previous.second
                state.trustStore = previous.third
                state.importedBackupVersion = previousImported
                throw failure
            }
        } catch (failure: Throwable) {
            updated.close()
            throw failure
        }
        trust.close()
        trust = updated
        // 新しい秘密を記録したので、鍵バックアップの今の版は検証し直す。
        backupState?.suspend()
    }

    /**
     * 鍵バックアップの送信が 409 `E2EE_SSSS_CONFLICT` になった。記録した `secrets_revision` を消して、
     * 解錠し直すまで、暗号ストアの鍵を使う処理（署名し直し、鍵バックアップの送信）を止める（TUI と同じ）。
     *
     * `failed` は、409 になった要求の `secrets_revision`。送っている間に解錠し直して新しい値を保存していたら
     * （値が違えば）、その解錠を取り消さない。
     */
    @Synchronized
    fun forgetSecretsRevision(failed: Long) {
        if (state.secretsRevision != failed) return
        backupState?.let {
            it.setSecretsRevision(null)
            it.suspend()
        }
        state.secretsRevision = null
        save()
    }

    /**
     * 暗号ストアの自分の鍵とサーバーの状態（自分だけの `keys/query` と `GET /api/e2ee/key_backup/versions`
     * の応答）から、起動時にすることを決める。まだ解錠していなければ `null`。
     */
    @Synchronized
    fun startupCheck(keysQuery: String, versions: String): StartupCheck? {
        val sealed = state.ownKeys ?: return null
        return OwnKeys.fromPickle(sealed, account, storageKey).use { it.startupCheck(keysQuery, versions, account.deviceKeys()) }
    }

    private fun loadBackupState(): BackupUploadState {
        backupState?.let { return it }
        val saved =
            state.backupUpload?.let {
                try {
                    BackupUploadState.fromJson(it, state.userId)
                } catch (_: ZooCryptoException) {
                    null
                }
            }
        return (saved ?: BackupUploadState(state.userId)).also { backupState = it }
    }

    /**
     * 保存した受信用セッションを、まだなら読み込む。読み込めたときだけ読み込み済みにする（壊れた pickle は、
     * 次の試行でもエラーになる。読めないセッションを送ったことにしない）。
     */
    private fun loadInbound(key: String) {
        if (key in loadedInbound) return
        state.inboundSessions[key]?.let { inbound.restoreSession(account, it.pickle, storageKey) }
        loadedInbound += key
    }

    /**
     * 鍵バックアップへ送る次の要求を作る（docs/e2ee.md の「鍵バックアップ」。TUI の `prepare_backup_upload`、
     * React の `prepareBackupUpload` と同じ規則）。
     *
     * `currentVersion` は `GET /api/e2ee/key_backup/versions/current` の応答（なければ `null`）。今の版の
     * `auth_data` に自分のマスター鍵の署名があり、公開鍵が SSSS のバックアップ鍵と同じで、前に見た版より
     * 古くないときだけ送る（`BackupUploadState.accept_current_version`）。版が変わったら、すべての
     * セッションを送り直す。
     */
    @Synchronized
    fun prepareBackupUpload(currentVersion: JsonObject?, limit: Int): BackupUploadRequest {
        // 送信は 1 つずつ（E2eeSetup.uploadKeyBackup）なので、ここで残っている要求は、送っている途中で
        // 捨てられたもの。記録を捨てる（入れたセッションは送信待ちのまま）。
        pendingUploads.values.forEach { it.upload.close() }
        pendingUploads.clear()
        if (state.ownKeys == null) return BackupUploadRequest.None
        if (currentVersion == null) {
            loadBackupState().suspend()
            return BackupUploadRequest.None
        }
        val version = try {
            acceptCurrentVersion(currentVersion)
        } catch (failure: ZooCryptoException) {
            return BackupUploadRequest.Rejected(failure.message.orEmpty())
        }
        val backup = loadBackupState()
        val keys = state.inboundSessions.filterValues { !it.backedUp }.keys.take(limit)
        if (keys.isEmpty()) return BackupUploadRequest.None
        // フィールドがない版（証明を足す前のサーバー）だけ、証明を付けずに送る。フィールドがあって鍵の形の
        // 文字列でなければ（`null`、数値、オブジェクトも）送らない。証明を黙って省いて、未証明の行を
        // 送信済みにしないため（Web・TUI と同じ）。
        val proof = when (val uploadKey = currentVersion["upload_key"]) {
            null -> null
            is JsonPrimitive -> if (!uploadKey.isString) return BackupUploadRequest.Rejected("upload_key が鍵の形ではありません")
                else try {
                    OwnKeys.fromPickle(checkNotNull(state.ownKeys), account, storageKey).use { own ->
                        own.backupKey().use { key -> key.uploadProof(uploadKey.content, userId, version) }
                    }
                } catch (failure: ZooCryptoException) {
                    return BackupUploadRequest.Rejected(failure.message.orEmpty())
                }
            else -> return BackupUploadRequest.Rejected("upload_key が鍵の形ではありません")
        }
        for (key in keys) {
            loadInbound(key)
            backup.enqueue(sessionRef(key))
        }
        val upload = backup.nextUpload(inbound, limit.toULong()) ?: return BackupUploadRequest.None
        val sent = upload.sessions().map(::inboundKey).toSet()
        val sessions = keys.filter { it in sent }.mapNotNull { key -> state.inboundSessions[key]?.let { key to it.pickle } }
        nextUploadId += 1
        val id = nextUploadId
        pendingUploads[id] = PendingUpload(upload, sessions)
        return BackupUploadRequest.Upload(
            id = id,
            version = upload.version(),
            secretsRevision = upload.secretsRevision().toLong(),
            body = json.parseToJsonElement(upload.body()) as JsonObject,
            count = sessions.size,
            proof = proof,
        )
    }

    /**
     * サーバーが今の版として返した版（`GET /api/e2ee/key_backup/versions/current` の応答）を、暗号ストアの自分の
     * マスター鍵とバックアップ鍵で検証して受け入れ（`BackupUploadState.accept_current_version`）、版の番号を返す。
     * 署名がない、公開鍵が違う、版が戻ったなどは [ZooCryptoException] を投げる。版が変わったら、手元の
     * セッションをすべて送信待ちに戻し、新しい版を見たことと同じ書き込みで記録する。
     */
    private fun acceptCurrentVersion(currentVersion: JsonObject): String {
        val sealed = checkNotNull(state.ownKeys) { "解錠していません" }
        val backup = loadBackupState()
        backup.setSecretsRevision(state.secretsRevision?.toULong())
        val change =
            OwnKeys.fromPickle(sealed, account, storageKey).use { own ->
                own.backupKey().use { key ->
                    backup.acceptCurrentVersion(currentVersion.toString(), own.masterPublicKey(), key)
                }
            }
        val previousSessions = state.inboundSessions
        val previousBackup = state.backupUpload
        if (change == VersionChange.CHANGED) {
            state.inboundSessions =
                state.inboundSessions.mapValues { (_, session) -> StoredInboundSession(session.pickle, backedUp = false) }
        }
        state.backupUpload = backup.toJson()
        try {
            save()
        } catch (failure: Throwable) {
            state.inboundSessions = previousSessions
            state.backupUpload = previousBackup
            // 新しい版を見たことを記録できなかったので、次の送信でもう一度戻す。
            backup.close()
            backupState = null
            throw failure
        }
        return checkNotNull(backup.currentVersion()).version
    }

    /**
     * 鍵バックアップから取り込む版を決める（新しい端末での解錠の手順 5）。解錠していない、今の版がない、
     * この版から取り込み終えているときは `null`。版を使えない（[acceptCurrentVersion] が拒んだ）ときは
     * [ZooCryptoException]。
     */
    @Synchronized
    fun backupVersionToImport(currentVersion: JsonObject?): String? {
        if (state.ownKeys == null || state.secretsRevision == null || currentVersion == null) return null
        val version = acceptCurrentVersion(currentVersion)
        return version.takeIf { it != state.importedBackupVersion }
    }

    /**
     * `GET /api/e2ee/key_backup/keys` の 1 ページの行を、暗号ストアのバックアップ鍵で復号して取り込む（docs/e2ee.md の
     * 「受信時の検証」の「鍵バックアップから取り込んだとき」）。復号と照合は zoo-crypto（`BackupDecryptionKey.import_rows`）
     * が行い、Megolm のセッション鍵は Kotlin に出さない。
     *
     * - バックアップ由来として保存し、Olm 由来のセッションを置き換えない（同じキーの保存済みのセッションを先に読み込み、
     *   両方を 1 つの pickle にして保存し直す）。
     * - 行と平文が合わない、`session_key` と `session_id` が合わない行は捨てる。
     * - 新しく取り込んだセッションは、この版から取ったので送信済みにする（同じ版に上げ直さない）。保存済みの
     *   セッションの送信済みの印は変えない。
     *
     * 返すのは取り込めた行の数。
     */
    @Synchronized
    fun importBackupRows(rows: List<JsonObject>): Int {
        val sealed = state.ownKeys ?: return 0
        if (rows.isEmpty()) return 0
        val refs =
            rows.mapNotNull { row ->
                fun text(name: String) = (row[name] as? JsonPrimitive)?.takeIf { it.isString }?.content
                val channel = text("channel_id") ?: return@mapNotNull null
                val sender = text("sender_key") ?: return@mapNotNull null
                val session = text("session_id") ?: return@mapNotNull null
                SessionRef(channel, sender, session)
            }
        refs.forEach { loadInbound(inboundKey(it)) }
        val discarded =
            OwnKeys.fromPickle(sealed, account, storageKey).use { own ->
                own.backupKey().use { key -> key.importRows(inbound, rows.map(JsonObject::toString)).toInt() }
            }
        val previous = state.inboundSessions
        var changed = previous
        for (ref in refs) {
            val key = inboundKey(ref)
            val pickle = inbound.pickleSession(account, ref, storageKey) ?: continue
            val existing = changed[key]
            if (existing?.pickle == pickle) continue
            changed = changed + (key to StoredInboundSession(pickle, backedUp = existing?.backedUp ?: true))
        }
        if (changed !== previous) {
            state.inboundSessions = changed
            try {
                save()
            } catch (failure: Throwable) {
                state.inboundSessions = previous
                throw failure
            }
        }
        return rows.size - discarded
    }

    /** `version` の鍵バックアップから最後まで取り込んだ。 */
    @Synchronized
    fun backupImported(version: String) {
        if (state.importedBackupVersion == version) return
        state.importedBackupVersion = version
        save()
    }

    /** [prepareBackupUpload] の `id` の要求が成功した。その要求に入れたセッションだけを送信済みにする。 */
    @Synchronized
    fun backupUploaded(id: Long) {
        val pending = pendingUploads.remove(id) ?: return
        pending.upload.use { upload ->
            backupState?.let {
                it.uploadSucceeded(upload)
                state.backupUpload = it.toJson()
            }
        }
        val sentPickles = pending.sessions.toMap()
        state.inboundSessions =
            state.inboundSessions.mapValues { (key, session) ->
                // 送ったあとに書き換わった（pickle が違う）セッションは、送信待ちのまま残す。
                if (sentPickles[key] == session.pickle) {
                    StoredInboundSession(session.pickle, backedUp = true)
                } else session
            }
        save()
    }

    /** `id` の要求が失敗した。入れたセッションは送信待ちのまま残す。409 のときは、今の版を検証し直すまで送らない。 */
    @Synchronized
    fun backupUploadFailed(id: Long, conflict: Boolean) {
        pendingUploads.remove(id)?.upload?.close()
        if (conflict) backupState?.suspend()
    }

    /** 終わっていない失効があるか（ログアウトの前に警告する。docs/e2ee.md の「失効の手順」）。 */
    @get:Synchronized
    val hasPendingRevocation: Boolean get() = state.revocation != null

    /** 終わっていない失効の進み具合（秘密鍵を除く。解錠する前の表示用）。なければ `null`。 */
    @Synchronized
    fun revocationSummary(): RevocationSummary? = state.revocation?.let { peekRevocation(it, account, storageKey) }

    /**
     * 失効を始め（署名し直す一覧を集め、新しい鍵を作る）、進み具合を保存する（サーバーにはまだ何も保存しない）。
     * 古いバックアップ鍵と古いユーザー署名鍵は、暗号ストアの自分の鍵のものを渡す（なければ SSSS の鍵を使う）。
     * 終わっていない失効があれば始めない（[IllegalStateException]）。
     */
    @Synchronized
    fun beginRevocation(input: RevocationInput, unlocked: UnlockedSecretStorage): Revocation {
        check(state.revocation == null) { "終わっていない失効があります" }
        val own = state.ownKeys?.let { OwnKeys.fromPickle(it, account, storageKey) }
        val revocation =
            try {
                val backupKey = own?.backupKey()
                val userSigning = own?.userSigning()
                try {
                    Revocation.begin(input, unlocked, trust, backupKey, userSigning)
                } finally {
                    backupKey?.close()
                    userSigning?.close()
                }
            } finally {
                own?.close()
            }
        try {
            saveRevocation(revocation)
        } catch (failure: Throwable) {
            revocation.close()
            throw failure
        }
        return revocation
    }

    /** 失効の進み具合を保存する（サーバーへの保存が 1 つ済むたびに呼ぶ）。書けなければ前の記録に戻す。 */
    @Synchronized
    fun saveRevocation(revocation: Revocation) {
        val previous = state.revocation
        state.revocation = revocation.pickle(account, storageKey)
        try {
            save()
        } catch (failure: Throwable) {
            state.revocation = previous
            throw failure
        }
    }

    /**
     * 保存した失効を、開け直した SSSS で続ける。なければ `null`。2〜5 を SSSS の保存の前に止めていたら
     * `ZooCryptoException.RevocationRestart`、SSSS の鍵が記録と違えば `ZooCryptoException.ServerKeyMismatch`。
     */
    @Synchronized
    fun resumeRevocation(unlocked: UnlockedSecretStorage): Revocation? =
        state.revocation?.let { Revocation.resume(it, account, storageKey, unlocked) }

    /** 失効の進み具合を消す（終えたとき、やり直すとき）。 */
    @Synchronized
    fun clearRevocation() {
        if (state.revocation == null) return
        val previous = state.revocation
        state.revocation = null
        try {
            save()
        } catch (failure: Throwable) {
            state.revocation = previous
            throw failure
        }
    }

    /** 今の端末への自己署名、署名し直すマスター鍵への署名、作り直すピン。 */
    @Synchronized
    fun revocationSignatures(revocation: Revocation): RevocationSignatures = revocation.signatures(account.deviceKeys())

    /**
     * 失効を行う端末が、クロス署名鍵を保存したあとに、記録と自分の鍵を新しい鍵にそろえる（Web の
     * `applyRevocation`）。新しい鍵の `secrets_revision` も記録する（次の起動で SSSS が書き換えられたと誤って
     * 判定しないため）。自分の鍵、`secrets_revision`、記録、失効の進み具合を 1 回の書き込みで保存し、書けなければ
     * メモリーの状態も前に戻して例外を投げる。
     */
    @Synchronized
    fun applyRevocation(revocation: Revocation, secretsRevision: Long?) {
        // 記録はコピーで更新し、すべて保存できたときだけ置き換える（saveOwnKeys と同じ）。
        val updated = TrustStore.fromJson(trust.toJson(), state.userId)
        try {
            revocation.applyToTrust(updated)
            val ownKeys = revocation.ownKeysPickle(account, storageKey)
            val trustStore = updated.pickle(account, storageKey)
            val sealed = revocation.pickle(account, storageKey)
            val previous = listOf(state.ownKeys, state.trustStore, state.revocation)
            val previousRevision = state.secretsRevision
            state.ownKeys = ownKeys
            state.secretsRevision = secretsRevision
            state.trustStore = trustStore
            state.revocation = sealed
            try {
                save()
            } catch (failure: Throwable) {
                state.ownKeys = previous[0]
                state.trustStore = previous[1]
                state.revocation = previous[2]
                state.secretsRevision = previousRevision
                throw failure
            }
        } catch (failure: Throwable) {
            updated.close()
            throw failure
        }
        trust.close()
        trust = updated
        // 自分の鍵が変わったので、鍵バックアップの今の版は検証し直す。
        backupState?.suspend()
    }

    /** 記録がある相手（失効で署名し直す候補を集めるため。Web の `trustRecordedUsers`）。 */
    @Synchronized
    fun trustRecordedUsers(): List<String> =
        ((json.parseToJsonElement(trust.toJson()) as? JsonObject)?.get("records") as? JsonObject)?.keys?.toList().orEmpty()

    /** 配布先の判定の記録（#361 以降が使う）。変えたら [saveTrustStore] で保存し直す。 */
    val trustStore: TrustStore get() = trust

    @Synchronized
    fun saveTrustStore() {
        state.trustStore = trust.pickle(account, storageKey)
        save()
    }

    /**
     * 暗号ストアを消さずに、端末だけを作り直す（サーバーから端末が消されたと確かめたとき。docs/e2ee.md の
     * 「端末が削除されていたとき」）。新しい端末アカウント（新しい `device_id`、ワンタイム鍵とフォールバック鍵。未登録）を
     * 作り、ユーザーに属する記録を引き継いだ新しい暗号ストアを返す。このストアは閉じる（ディレクトリーの印は、
     * 返すストアに引き継ぐ）。
     *
     * - 引き継ぐ：配布先の判定の記録（TrustStore）と、受信用セッションのすべて（鍵バックアップの送信待ちの印も
     *   そのまま）。どれも封の AAD に `device_id` が入るので、保存したものを古い端末アカウントで開き、新しい端末
     *   アカウントで封をし直す。開けない受信用セッション（壊れたもの）は捨てる。鍵バックアップの送信の状態
     *   （見た版の最大。版の巻き戻しの検出に使う）も残す。
     * - 捨てる：Olm セッション（古い端末の鍵に結び付く）、処理済みの to-device の ID、解錠した自分の鍵と
     *   `secrets_revision`（利用者が解錠し直す。別の端末からわざと消された端末が、自分の鍵で新しい端末に勝手に
     *   署名しないため）、鍵バックアップからの取り込みの進み具合。
     *
     * `state.json` は 1 回の書き込みで置き換えるので、途中で止まっても、前の端末か新しい端末のどちらかが残る。
     * 書けなければ、このストアはそのまま（開いたまま）にして例外を投げる。古い端末の削除 API は呼ばない
     * （サーバーはもう消したと言っている）。
     */
    @Synchronized
    fun replaceDevice(): CryptoStore {
        check(!closed) { "暗号ストアは閉じています" }
        val opened = mutableListOf<AutoCloseable>()
        val key = storageKey.copyOf()
        try {
            val newAccount = DeviceAccount(state.userId, UUID.randomUUID().toString()).also(opened::add)
            newAccount.generateOneTimeKeys(newAccount.maxNumberOfOneTimeKeys())
            newAccount.generateFallbackKey()
            // 保存したもの（メモリーの記録ではなく）を、古い端末で開いて新しい端末で封をし直す。
            val newTrust =
                (state.trustStore?.let { TrustStore.fromPickle(it, account, key) } ?: TrustStore(state.userId))
                    .also(opened::add)
            val sessions = InboundSessionStore().also(opened::add)
            val resealed = mutableMapOf<String, StoredInboundSession>()
            for ((_, stored) in state.inboundSessions) {
                val ref =
                    try {
                        sessions.restoreSession(account, stored.pickle, key)
                    } catch (_: ZooCryptoException) {
                        // 開けない（壊れた）セッションは捨てる。
                        continue
                    }
                val pickle = sessions.pickleSession(newAccount, ref, key) ?: continue
                resealed[inboundKey(ref)] = StoredInboundSession(pickle, backedUp = stored.backedUp)
            }
            // 見た版の最大と送信待ちは残し、解錠とともに捨てる secrets_revision だけを外す。
            val backupUpload =
                state.backupUpload?.let { saved ->
                    try {
                        BackupUploadState.fromJson(saved, state.userId).use {
                            it.setSecretsRevision(null)
                            it.toJson()
                        }
                    } catch (_: ZooCryptoException) {
                        null
                    }
                }
            val next =
                StoreState(
                    version = STATE_VERSION,
                    userId = state.userId,
                    deviceId = newAccount.deviceId(),
                    account = newAccount.pickle(key),
                    inboundSessions = resealed,
                    trustStore = newTrust.pickle(newAccount, key),
                    backupUpload = backupUpload,
                )
            val replacement =
                CryptoStore(
                    directory, key, newAccount, OlmSessionStore().also(opened::add), sessions, newTrust, next,
                    deleteWrappingKey,
                )
            replacement.save()
            // 新しいストアに引き継いだので、ディレクトリーの印は外さずに、このストアのものだけを手放す。
            releaseResources()
            return replacement
        } catch (failure: Throwable) {
            opened.forEach { it.close() }
            key.fill(0)
            throw failure
        }
    }

    /**
     * 暗号ストアを消す（ログアウト）。先に保存鍵を包む鍵を消し、失敗してもファイルの削除を試す
     * （React の `destroyCryptoStore` と同じ順）。消し切れなかったら最初の例外を投げる。
     *
     * 消し終えるまでディレクトリーを開いたままの印にしておき、ほかの処理がそのあいだに開き直した
     * ストアを消さないようにする（静的な [destroy] と同じ）。
     */
    @Synchronized
    fun destroy() {
        val path = directory.canonicalPath
        // 閉じたあとなら、印を取り直す（ほかの処理が開いていれば消さない）。
        if (closed && !openDirectories.add(path)) throw CryptoStoreBusyException()
        try {
            releaseResources()
            destroyFiles(directory, deleteWrappingKey)
        } finally {
            openDirectories.remove(path)
        }
    }

    @Synchronized
    override fun close() {
        if (closed) return
        releaseResources()
        openDirectories.remove(directory.canonicalPath)
    }

    /** 保存鍵と FFI のオブジェクトを手放す。ディレクトリーの印は外さない。 */
    private fun releaseResources() {
        if (closed) return
        closed = true
        storageKey.fill(0)
        pendingUploads.values.forEach { it.upload.close() }
        pendingUploads.clear()
        backupState?.close()
        backupState = null
        account.close()
        olmSessions.close()
        inbound.close()
        trust.close()
    }

    private fun save() {
        check(!closed) { "暗号ストアは閉じています" }
        writeAtomically(File(directory, STATE_FILE), json.encodeToString(StoreState.serializer(), state).encodeToByteArray())
        changes.value++
    }

    companion object {
        private const val STATE_VERSION = 1
        private const val STATE_FILE = "state.json"
        private const val KEY_FILE = "storage.key"
        internal val json = Json {
            ignoreUnknownKeys = true
            encodeDefaults = true
        }

        /** 開いている暗号ストアのディレクトリー（プロセスの中で同時に 2 つ開かないため）。 */
        private val openDirectories: MutableSet<String> = ConcurrentHashMap.newKeySet()

        /**
         * `userId` の暗号ストアを開く。なければ（`create` が真なら）端末を作る（`device_id` は UUID v4）。
         * `create` が偽でなければ `null` を返す（ログアウトで、なければ作らずに済ませるため）。
         *
         * 保存鍵を失った（ファイルがない、包む鍵が消えた・失効した、壊れた）ときは、どの値も読めないので
         * 作り直して新しい端末にする。前の端末の ID と登録済みか（平文）は `state.json` にしかないので、
         * 消す前に [rememberReplaced] に渡す（削除待ちに残し、作り直しが途中で失敗しても消せるように）。
         * [rememberReplaced] が例外を投げたら、何も消さずに投げる。
         *
         * Keystore の一時的な失敗はそのまま投げる（作り直すと、Olm セッションや、まだバックアップして
         * いない受信用セッションが消えるため）。
         */
        fun open(
            files: CryptoStoreFiles,
            userId: String,
            create: Boolean = true,
            rememberReplaced: (deviceId: String, registered: Boolean) -> Unit = { _, _ -> },
        ): Opened? {
            val path = files.directory.canonicalPath
            if (!openDirectories.add(path)) throw CryptoStoreBusyException()
            try {
                return openUnlocked(files, userId, create, rememberReplaced).also {
                    if (it == null) openDirectories.remove(path)
                }
            } catch (failure: Throwable) {
                openDirectories.remove(path)
                throw failure
            }
        }

        private fun openUnlocked(
            files: CryptoStoreFiles,
            userId: String,
            create: Boolean,
            rememberReplaced: (deviceId: String, registered: Boolean) -> Unit,
        ): Opened? {
            val directory = files.directory
            val stateFile = File(directory, STATE_FILE)
            val keyFile = File(directory, KEY_FILE)
            val keys = StorageKeyStore(keyFile, files.wrappingKey)
            var replaced: String? = null
            val stored = readState(stateFile)
            if (stored != null) {
                check(stored.version == STATE_VERSION) { "暗号ストアの版 ${stored.version} には対応していません" }
                check(stored.userId == userId) { "暗号ストアが別のユーザーのものです" }
                val key =
                    try {
                        keys.loadExisting()
                    } catch (_: StorageKeyUnavailableException) {
                        null
                    }
                if (key != null) return Opened(restore(files, key, stored), null)
                // 保存鍵がなければ、どの値も読めない。作り直して、新しい端末として登録し直す。
                rememberReplaced(stored.deviceId, stored.registered)
                replaced = stored.deviceId
                if (!stateFile.delete() && stateFile.exists()) throw IOException("暗号ストアを消せません")
                discardStorageKey(keyFile, files)
            }
            if (!create) return null
            directory.mkdirs()
            val storageKey =
                try {
                    keys.loadOrCreate()
                } catch (_: StorageKeyUnavailableException) {
                    // 状態はなく、開けない保存鍵だけが残っている。作り直す。
                    discardStorageKey(keyFile, files)
                    keys.loadOrCreate()
                }
            val opened = mutableListOf<AutoCloseable>()
            try {
                val account = DeviceAccount(userId, UUID.randomUUID().toString()).also(opened::add)
                account.generateOneTimeKeys(account.maxNumberOfOneTimeKeys())
                account.generateFallbackKey()
                val trust = TrustStore(userId).also(opened::add)
                val state =
                    StoreState(
                        version = STATE_VERSION,
                        userId = userId,
                        deviceId = account.deviceId(),
                        account = account.pickle(storageKey),
                        trustStore = trust.pickle(account, storageKey),
                    )
                val store =
                    CryptoStore(
                        directory, storageKey, account, OlmSessionStore().also(opened::add),
                        InboundSessionStore().also(opened::add), trust, state, files.deleteWrappingKey,
                    )
                store.save()
                return Opened(store, replaced)
            } catch (failure: Throwable) {
                opened.forEach { it.close() }
                storageKey.fill(0)
                throw failure
            }
        }

        private fun restore(files: CryptoStoreFiles, storageKey: ByteArray, state: StoreState): CryptoStore {
            val opened = mutableListOf<AutoCloseable>()
            try {
                val account =
                    DeviceAccount.fromPickle(state.account, storageKey, state.userId, state.deviceId).also(opened::add)
                val olmSessions = OlmSessionStore().also(opened::add)
                state.olmSessions.forEach { olmSessions.restoreSession(account, it.pickle, storageKey) }
                val trust =
                    (state.trustStore?.let { TrustStore.fromPickle(it, account, storageKey) } ?: TrustStore(state.userId))
                        .also(opened::add)
                return CryptoStore(
                    files.directory, storageKey, account, olmSessions, InboundSessionStore(), trust, state, files.deleteWrappingKey,
                )
            } catch (failure: Throwable) {
                opened.forEach { it.close() }
                storageKey.fill(0)
                throw failure
            }
        }

        private fun readState(file: File): StoreState? {
            if (!file.exists()) return null
            return json.decodeFromString(StoreState.serializer(), file.readText())
        }

        private fun discardStorageKey(keyFile: File, files: CryptoStoreFiles) {
            if (!keyFile.delete() && keyFile.exists()) throw IOException("保存鍵のファイルを消せません")
            // 失効した Keystore の鍵が残ると、作り直しても同じ鍵で包んでしまうので消す。
            files.deleteWrappingKey()
        }

        /**
         * 平文の `device_id` を読む（暗号ストアを開けないときでも、ログアウトでサーバーから消すため）。
         * なければ、または別のユーザーのものなら `null`。
         */
        fun storedDeviceId(files: CryptoStoreFiles, userId: String): String? =
            try {
                readState(File(files.directory, STATE_FILE))?.takeIf { it.userId == userId }?.deviceId
            } catch (_: Exception) {
                null
            }

        /**
         * 開けなかった暗号ストアを消す（ログアウト）。開いているあいだは [CryptoStoreBusyException]。
         */
        fun destroy(files: CryptoStoreFiles) {
            val path = files.directory.canonicalPath
            if (!openDirectories.add(path)) throw CryptoStoreBusyException()
            try {
                destroyFiles(files.directory, files.deleteWrappingKey)
            } finally {
                openDirectories.remove(path)
            }
        }

        private fun destroyFiles(directory: File, deleteWrappingKey: () -> Unit) {
            var failure: Throwable? = null
            try {
                deleteWrappingKey()
            } catch (e: Exception) {
                failure = e
            }
            if (!directory.deleteRecursively()) {
                val error = IOException("暗号ストアを消せません")
                failure?.addSuppressed(error) ?: run { failure = error }
            }
            failure?.let { throw it }
        }

        /** 受信用セッションのキーを 1 つの文字列にする（TUI と同じ形）。 */
        internal fun inboundKey(ref: SessionRef): String =
            JsonArray(listOf(JsonPrimitive(ref.channelId), JsonPrimitive(ref.senderKey), JsonPrimitive(ref.sessionId)))
                .toString()

        /** [inboundKey] の逆。 */
        internal fun sessionRef(key: String): SessionRef {
            val parts = (json.parseToJsonElement(key) as JsonArray).map { (it as JsonPrimitive).content }
            check(parts.size == 3) { "受信用セッションのキーを読めません" }
            return SessionRef(parts[0], parts[1], parts[2])
        }

        /**
         * 一時ファイルに書いて `fsync` してから置き換え、親ディレクトリーも同期する。どれかに失敗したら
         * 例外を投げる（保存できたことにして ack を送らないため）。
         */
        internal fun writeAtomically(target: File, bytes: ByteArray) {
            target.parentFile?.mkdirs()
            val temporary = File(target.parentFile, "${target.name}.tmp")
            FileOutputStream(temporary).use { output ->
                output.write(bytes)
                output.fd.sync()
            }
            Files.move(
                temporary.toPath(), target.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING,
            )
            // rename したディレクトリーのエントリーも書き出す（停電で前の状態に戻らないように）。
            directorySync(target.parentFile!!)
        }

        /**
         * ディレクトリーを `fsync` する。JVM（単体テスト）では `FileChannel` で開けるが、Android の
         * `FileChannel.open` はディレクトリーを開けない（EISDIR）ので、`android.system.Os` で開く。
         */
        /** ディレクトリーの同期。単体テストで失敗を差し込むために差し替えられる。 */
        internal var directorySync: (File) -> Unit = ::syncDirectory

        private fun syncDirectory(directory: File) {
            val channelFailure =
                try {
                    FileChannel.open(directory.toPath(), StandardOpenOption.READ).use { it.force(true) }
                    return
                } catch (e: IOException) {
                    e
                }
            try {
                val fd = android.system.Os.open(directory.path, android.system.OsConstants.O_RDONLY, 0)
                try {
                    android.system.Os.fsync(fd)
                } finally {
                    android.system.Os.close(fd)
                }
            } catch (e: android.system.ErrnoException) {
                throw IOException("${directory.name} を同期できません", e).apply { addSuppressed(channelFailure) }
            }
        }
    }
}
