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
)

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
     * 送信端末の検証（受信時の検証の手順 4）は、今は自分の別の端末だけを行う（解錠済みなら、暗号ストアの自分の
     * 自己署名鍵と、その鍵による端末鍵への署名で確かめる）。他のユーザーの端末は、配布先の判定の記録で信頼した
     * マスター鍵が要るので、送受信（#361）で入れる。それまでは検証しない（`trust` を渡さない）。
     */
    @Synchronized
    fun processToDevice(
        rows: List<ToDeviceRow>,
        deviceKeys: JsonObject,
        signatures: JsonArray = JsonArray(emptyList()),
    ): ToDeviceResult {
        val processed = mutableListOf<String>()
        var roomKeys = 0
        var dropped = 0
        var newOlmSessions = 0
        var verifiedRoomKeys = 0
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
            selfSigning?.let { key ->
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
        return ToDeviceResult(processed, roomKeys, dropped, newOlmSessions, verifiedRoomKeys)
    }

    private sealed interface Encrypted {
        data object Dropped : Encrypted
        data class Decrypted(
            val newOlmSessions: Int,
            val roomKey: Boolean,
            val rejected: Boolean,
            val verified: Boolean = false,
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
                if (loadedInbound.add(key)) {
                    state.inboundSessions[key]?.let { inbound.restoreSession(account, it.pickle, storageKey) }
                }
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
        try {
            acceptCurrentVersion(currentVersion)
        } catch (failure: ZooCryptoException) {
            return BackupUploadRequest.Rejected(failure.message.orEmpty())
        }
        val backup = loadBackupState()
        val keys = state.inboundSessions.filterValues { !it.backedUp }.keys.take(limit)
        if (keys.isEmpty()) return BackupUploadRequest.None
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
        state.inboundSessions =
            state.inboundSessions.mapValues { (key, session) ->
                // 送ったあとに書き換わった（pickle が違う）セッションは、送信待ちのまま残す。
                if (pending.sessions.any { it.first == key && it.second == session.pickle }) {
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

    /** 配布先の判定の記録（#361 以降が使う）。変えたら [saveTrustStore] で保存し直す。 */
    val trustStore: TrustStore get() = trust

    @Synchronized
    fun saveTrustStore() {
        state.trustStore = trust.pickle(account, storageKey)
        save()
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
