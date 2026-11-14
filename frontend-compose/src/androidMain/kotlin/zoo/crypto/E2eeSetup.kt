package zoo.crypto

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull
import zoo.ApiException
import zoo.crypto.ffi.PasskeySlot
import zoo.crypto.ffi.SetupSession
import zoo.crypto.ffi.StartupCheck
import zoo.crypto.ffi.ZooCryptoException
import zoo.crypto.ffi.parseSecretStorage

/**
 * 初回設定と解錠で使う E2EE の API（docs/e2ee.md の「クロス署名」「Secret Storage」「鍵バックアップ」。#360）。
 * React の `secretStorageApi.ts`、`client-core` の `api/e2ee.rs` と同じ。
 *
 * 直前の再認証が要る要求には、`reauth` に再認証で受け取った ID トークンを渡す（`X-Zoo-Reauth`）。
 */
interface E2eeSetupApi {
    /** `GET /api/e2ee/secret_storage`。SSSS がまだなければ `default_key_id` が `null`。 */
    suspend fun secretStorage(): JsonObject

    /** `POST /api/e2ee/secret_storage/keys`。増やしたあとの `secrets_revision` を返す。 */
    suspend fun createSecretStorageKey(body: JsonObject, reauth: String?): Long?

    /** `POST /api/e2ee/secret_storage/slots`。 */
    suspend fun addKeySlot(slot: JsonObject, reauth: String?)

    /** `POST /api/e2ee/secret_storage/slots/{slot_id}/used`。表示用の記録なので、失敗は無視する。 */
    suspend fun markKeySlotUsed(slotId: String)

    /** 自分だけを問い合わせた `POST /api/e2ee/keys/query`（クロス署名鍵と署名を含む応答）。 */
    suspend fun queryOwnKeys(userId: String): JsonObject

    /** `GET /api/e2ee/key_backup/versions`（消した版を含むすべての版）。 */
    suspend fun backupVersions(): JsonObject

    /** `GET /api/e2ee/key_backup/versions/current`。今の版がなければ `null`。 */
    suspend fun currentBackupVersion(): JsonObject?

    /** `POST /api/e2ee/cross_signing/keys`。 */
    suspend fun uploadCrossSigningKeys(body: JsonObject, reauth: String?)

    /** `POST /api/e2ee/cross_signing/signatures`。 */
    suspend fun uploadSignatures(signatures: JsonArray)

    /** `POST /api/e2ee/key_backup/versions`。 */
    suspend fun createBackupVersion(body: JsonObject, reauth: String?)

    /** `POST /api/e2ee/key_backup/keys?version=<版>&secrets_revision=<数>`。 */
    suspend fun uploadBackupKeys(version: String, secretsRevision: Long, body: JsonObject)
}

/** 今の状態（React の `E2eeStatus`、TUI の `E2eeStatus` と同じ）。 */
sealed interface E2eeStatus {
    /** SSSS がまだない。初回設定を始める。 */
    data object NotSetUp : E2eeStatus

    /** SSSS はあるが、この端末は解錠していない。 */
    data class Locked(val reason: LockReason, val storage: JsonObject) : E2eeStatus

    /** 解錠済み。 */
    data class Unlocked(val storage: JsonObject) : E2eeStatus
}

enum class LockReason {
    /** まだ解錠していない（新しい端末）。 */
    NEW,

    /** 解錠済みの端末が、初回設定の途中で止まったのを見つけた。SSSS を開け直して続ける。 */
    RESUME,

    /** 暗号ストアの鍵とサーバーの鍵が違う（失効のあとか、改ざん）。 */
    MISMATCH,
}

/** 初回設定と解錠の誤り。 */
sealed class E2eeSetupException(message: String) : Exception(message) {
    /** 直前の再認証が要る（403 `E2EE_REAUTH_REQUIRED`）。同じ鍵のまま、再認証してやり直す。 */
    class ReauthRequired : E2eeSetupException("再認証が必要です")

    /** 別の端末が先に初回設定を始めた（409 `E2EE_SSSS_CONFLICT`）。手元の鍵を捨てる。表示したリカバリーキーは使えない。 */
    class Conflict : E2eeSetupException("別の端末が先に初回設定を始めました")

    /** サーバーの鍵が SSSS の鍵と違う（失効が途中か、SSSS が書き換えられた）。 */
    class ServerKeyMismatch(message: String) : E2eeSetupException(message)

    /** SSSS がまだない（解錠するものがない）。 */
    class NoSecretStorage : E2eeSetupException("暗号化はまだ設定されていません")
}

/** [E2eeSetup.uploadKeyBackup] の結果。 */
sealed interface BackupUploadResult {
    data class Done(val uploaded: Int) : BackupUploadResult

    /** 今の版を使えない（署名がない、SSSS の鍵と違う、巻き戻りなど）。送っていない。 */
    data class Rejected(val reason: String) : BackupUploadResult

    /** サーバーが拒んだ（409 `E2EE_WRONG_BACKUP_VERSION` か `E2EE_SSSS_CONFLICT`）。 */
    data class Conflict(val code: String) : BackupUploadResult
}

/** [E2eeSetup.unlock] の結果。 */
class Unlocked(
    /** 開けた SSSS（続きの保存に使う）。使い終わったら `close()` する。 */
    val session: SetupSession,
    val secretsRevision: Long?,
    /** 続きの保存（初回設定の再開でクロス署名鍵がない）に再認証が要る。`false` なら最後まで保存してある。 */
    val needsReauth: Boolean,
)

/** 解錠の手段。 */
sealed interface UnlockMethod {
    data class Recovery(val recoveryKey: String) : UnlockMethod

    /** パスキー。`rpId` は issuer のホスト名。 */
    data class Passkey(val client: PasskeyClient, val rpId: String) : UnlockMethod
}

fun ApiException.code(): String? = (body?.get("code") as? JsonPrimitive)?.contentOrNull

fun isReauthRequired(failure: Throwable): Boolean =
    failure is ApiException && failure.status == 403 && failure.code() == "E2EE_REAUTH_REQUIRED"

fun isSsssConflict(failure: Throwable): Boolean =
    failure is ApiException && failure.status == 409 && failure.code() == "E2EE_SSSS_CONFLICT"

/** SSSS があるか（`default_key_id` が空でない）。 */
fun hasSecretStorage(storage: JsonObject): Boolean =
    (storage["default_key_id"] as? JsonPrimitive)?.contentOrNull?.isNotEmpty() == true

private fun JsonObject.defaultKeyId(): String? = (this["default_key_id"] as? JsonPrimitive)?.contentOrNull

private fun JsonObject.secretsRevision(): Long? = (this["secrets_revision"] as? JsonPrimitive)?.longOrNull

private fun JsonObject.slots(): List<JsonObject> = (this["slots"] as? JsonArray)?.mapNotNull { it as? JsonObject }.orEmpty()

private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull

private fun parseObject(value: String): JsonObject = CryptoStore.json.parseToJsonElement(value) as JsonObject

/**
 * 初回設定と解錠の手順（docs/e2ee.md の「初回設定と解錠の流れ」。#360）。React の `setupController.ts`、
 * TUI の `e2ee/setup.rs` と同じ流れで、保存の順と冪等の扱いを合わせる。
 *
 * 秘密（マスター鍵の秘密鍵、SSSS 鍵）は UniFFI の [SetupSession]（Rust のメモリー）にだけ置き、ここでは
 * それが作った要求の本文をサーバーに送る。暗号ストアは [store] で受け取る（端末の管理が開いているものを
 * 共有する。[CryptoStore] のメソッドは同期されている）。
 */
class E2eeSetup(
    private val api: E2eeSetupApi,
    val userId: String,
    private val store: suspend () -> CryptoStore,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) {
    /** 鍵バックアップへの送信を 1 つずつにする（呼び出し元が端末の管理でも画面でも、送信を重ねない）。 */
    private val backupUpload = Mutex()

    private suspend fun serverState(): Pair<String, String> = coroutineScope {
        val keys = async { api.queryOwnKeys(userId) }
        val versions = async { api.backupVersions() }
        keys.await().toString() to versions.await().toString()
    }

    /** 今の状態を読む。解錠済みの端末は、起動時の確認（署名し直し、止まった初回設定の発見）も行う。 */
    suspend fun loadStatus(): E2eeStatus = startupStatus(store(), api.secretStorage())

    /**
     * `GET /api/e2ee/secret_storage` の応答と暗号ストアから状態を決め、要るなら自分の端末に署名し直す。
     *
     * 解錠済みの端末でも、最後に解錠したときの `secrets_revision` がサーバーの値と違えば、暗号ストアの鍵を
     * 使わずに解錠し直しを求める（TUI と同じ。古い値のまま鍵バックアップに送り続けないため）。
     */
    private suspend fun startupStatus(store: CryptoStore, storage: JsonObject): E2eeStatus {
        if (!hasSecretStorage(storage)) return E2eeStatus.NotSetUp
        if (!store.hasOwnKeys) return E2eeStatus.Locked(LockReason.NEW, storage)
        if (storage.secretsRevision() != store.secretsRevision) return E2eeStatus.Locked(LockReason.MISMATCH, storage)
        val (keys, versions) = serverState()
        return when (val check = withContext(io) { store.startupCheck(keys, versions) }) {
            null -> E2eeStatus.Locked(LockReason.NEW, storage)
            is StartupCheck.Ready -> E2eeStatus.Unlocked(storage)
            is StartupCheck.SignDevice -> {
                api.uploadSignatures(JsonArray(listOf(CryptoStore.json.parseToJsonElement(check.signature))))
                E2eeStatus.Unlocked(storage)
            }
            is StartupCheck.ResumeSetup -> E2eeStatus.Locked(LockReason.RESUME, storage)
            is StartupCheck.Unlock -> E2eeStatus.Locked(LockReason.MISMATCH, storage)
        }
    }

    /**
     * 起動時の確認（docs/e2ee.md の「初回設定と解錠の流れ」）。解錠済みの端末だけが、自分の端末への署名を
     * 確かめ、なければ署名し直し、送信待ちのセッションを鍵バックアップに送る。まだ解錠していない端末は、
     * 何も取得しない。解錠し直しが要る端末（`secrets_revision` が変わった、鍵が違う）は送らない。
     */
    suspend fun runStartupCheck(): E2eeStatus? {
        val store = store()
        if (!store.hasOwnKeys) return null
        val status = startupStatus(store, api.secretStorage())
        if (status is E2eeStatus.Unlocked) uploadKeyBackup()
        return status
    }

    /**
     * 鍵バックアップの送信待ちのセッションを、今の版に送る（docs/e2ee.md の「鍵バックアップ」）。
     * 解錠していない端末と、今の版がない場合は何もしない。送信は 1 つずつ行う。
     */
    suspend fun uploadKeyBackup(): BackupUploadResult =
        backupUpload.withLock {
            val store = store()
            if (!store.hasOwnKeys) return@withLock BackupUploadResult.Done(0)
            val current = api.currentBackupVersion()
            var uploaded = 0
            while (true) {
                val request =
                    when (val prepared = withContext(io) { store.prepareBackupUpload(current, BACKUP_UPLOAD_BATCH) }) {
                        is BackupUploadRequest.None -> return@withLock BackupUploadResult.Done(uploaded)
                        is BackupUploadRequest.Rejected -> return@withLock BackupUploadResult.Rejected(prepared.reason)
                        is BackupUploadRequest.Upload -> prepared
                    }
                try {
                    api.uploadBackupKeys(request.version, request.secretsRevision, request.body)
                } catch (failure: Throwable) {
                    val code = (failure as? ApiException)?.takeIf { it.status == 409 }?.code()
                    val conflict = code == "E2EE_WRONG_BACKUP_VERSION" || code == "E2EE_SSSS_CONFLICT"
                    withContext(io) {
                        store.backupUploadFailed(request.id, conflict)
                        // SSSS の秘密が書き換えられた（失効など）。解錠し直すまで送らない。
                        if (code == "E2EE_SSSS_CONFLICT") store.forgetSecretsRevision()
                    }
                    if (conflict) return@withLock BackupUploadResult.Conflict(code)
                    throw failure
                }
                withContext(io) { store.backupUploaded(request.id) }
                uploaded += request.count
            }
            @Suppress("UNREACHABLE_CODE")
            BackupUploadResult.Done(uploaded)
        }

    /** 初回設定を始める。返したリカバリーキーを利用者に控えてもらう。 */
    fun begin(): zoo.crypto.ffi.SetupStart = zoo.crypto.ffi.beginSetup(userId)

    /** 初回設定に、パスキー（PRF）のスロットを足す。 */
    suspend fun addSetupPasskey(session: SetupSession, client: PasskeyClient, rpId: String, label: String?) {
        val salt = zoo.crypto.ffi.newPrfSalt()
        val result = client.prfForNewSlot(rpId, salt)
        try {
            session.addPasskeySlot(result.credentialId, salt, result.prfOutput, label)
        } finally {
            result.prfOutput.fill(0)
        }
    }

    /**
     * 初回設定を保存する。SSSS、クロス署名鍵、端末への署名、バックアップの版の順。
     *
     * - 403 `E2EE_REAUTH_REQUIRED` なら [E2eeSetupException.ReauthRequired]。同じ鍵のまま再認証してやり直す。
     * - SSSS の保存が 409 `E2EE_SSSS_CONFLICT` なら、読み直して、自分の鍵が保存済みなら続ける。別の鍵なら
     *   [E2eeSetupException.Conflict]（手元の鍵を捨てる。表示したリカバリーキーは使えない）。
     */
    suspend fun completeSetup(session: SetupSession, reauth: String?) {
        val body = parseObject(session.secretStorageRequest())
        val revision =
            try {
                api.createSecretStorageKey(body, reauth)
            } catch (failure: Throwable) {
                if (!isSsssConflict(failure)) throw mapReauth(failure)
                // 応答をなくして送り直したか、別の端末が先に保存した。読み直して判定する。
                val current = api.secretStorage()
                if (current.defaultKeyId() != session.keyId()) throw E2eeSetupException.Conflict()
                // 前の試行で SSSS を保存したあとに足したスロットは、まだサーバーにない（同じ ID の再送は何もしない）。
                val saved = current.slots().mapNotNull { it.text("id") }.toSet()
                for (slot in session.pendingSlots().map(::parseObject)) {
                    if (slot.text("id") !in saved) {
                        try {
                            api.addKeySlot(slot, reauth)
                        } catch (failure: Throwable) {
                            throw mapReauth(failure)
                        }
                    }
                }
                current.secretsRevision()
            }
        saveRemaining(session, revision, reauth)
    }

    /**
     * SSSS を保存したあと（または解錠したあと）の続きを保存し、自分の鍵を暗号ストアに保存する。応答をなくして
     * 送り直しても、同じ鍵なら成功する（サーバーの冪等の規則）。続きの保存は、作った（開けた）SSSS 鍵が
     * サーバーの既定の鍵のままであることを前提にする（`expected_default_key_id`）。
     */
    suspend fun saveRemaining(session: SetupSession, secretsRevision: Long?, reauth: String?) {
        val plan = planRemaining(session)
        val keyId = session.keyId()
        fun withPrecondition(body: String) = JsonObject(parseObject(body) + ("expected_default_key_id" to JsonPrimitive(keyId)))
        try {
            plan.crossSigning?.let { api.uploadCrossSigningKeys(withPrecondition(it), reauth) }
            plan.deviceSignature?.let { api.uploadSignatures(JsonArray(listOf(CryptoStore.json.parseToJsonElement(it)))) }
            plan.backupVersion?.let { api.createBackupVersion(withPrecondition(it), reauth) }
        } catch (failure: Throwable) {
            if (isSsssConflict(failure)) throw E2eeSetupException.Conflict()
            throw mapReauth(failure)
        }
        val store = store()
        withContext(io) { store.saveOwnKeys(session, secretsRevision) }
        // 解錠した（または初回設定を終えた）ので、手元のセッションを今の版に送る。失敗しても解錠は済んでいる。
        try {
            uploadKeyBackup()
        } catch (failure: kotlinx.coroutines.CancellationException) {
            throw failure
        } catch (_: Exception) {
        }
    }

    private suspend fun planRemaining(session: SetupSession): zoo.crypto.ffi.ResumePlan {
        val (state, storage) = coroutineScope {
            val state = async { serverState() }
            val storage = async { api.secretStorage() }
            state.await() to storage.await()
        }
        // 別の端末が SSSS を作り直した（同時の初回設定など）。手元の鍵でクロス署名鍵や版を差し替えると、
        // SSSS とサーバーの鍵の世代がずれるので、何も送らない（読んでから送るまでの間の競合は、
        // `expected_default_key_id` でサーバーが防ぐ）。
        if (storage.defaultKeyId() != session.keyId()) throw E2eeSetupException.Conflict()
        val deviceKeys = withContext(io) { store().deviceKeys() }
        return try {
            session.plan(state.first, state.second, deviceKeys)
        } catch (failure: ZooCryptoException.ServerKeyMismatch) {
            throw E2eeSetupException.ServerKeyMismatch(failure.message.orEmpty())
        }
    }

    /**
     * パスキーかリカバリーキーで SSSS を開ける。続きの保存に再認証が要らなければ、そのまま最後まで保存する。
     * 要るとき（初回設定の再開でクロス署名鍵がない）は `needsReauth` を真にして返すので、利用者の操作で
     * 再認証してから [saveRemaining] を呼ぶ。
     */
    suspend fun unlock(method: UnlockMethod): Unlocked {
        val storage = api.secretStorage()
        if (!hasSecretStorage(storage)) throw E2eeSetupException.NoSecretStorage()
        val (session, slotId) =
            when (method) {
                is UnlockMethod.Recovery -> {
                    val session =
                        zoo.crypto.ffi.unlockSetupWithRecoveryKey(userId, storage.toString(), method.recoveryKey)
                            ?: throw E2eeSetupException.NoSecretStorage()
                    session to storage.slots().firstOrNull { it.text("kind") == "recovery" }?.text("id")
                }
                is UnlockMethod.Passkey -> unlockWithPasskey(storage, method.client, method.rpId)
            }
        try {
            slotId?.let { api.markKeySlotUsed(it) }
            val revision = session.secretsRevision()?.toLong()
            val plan = planRemaining(session)
            if (plan.crossSigning != null) return Unlocked(session, revision, needsReauth = true)
            saveRemaining(session, revision, null)
            return Unlocked(session, revision, needsReauth = false)
        } catch (failure: Throwable) {
            session.close()
            throw failure
        }
    }

    private suspend fun unlockWithPasskey(
        storage: JsonObject,
        client: PasskeyClient,
        rpId: String,
    ): Pair<SetupSession, String> {
        val parsed = parseSecretStorage(userId, storage.toString()) ?: throw E2eeSetupException.NoSecretStorage()
        parsed.use { secretStorage ->
            val slots = secretStorage.passkeySlots()
            if (slots.isEmpty()) throw PasskeyException(PasskeyError.NO_SLOT, "パスキーのスロットがありません")
            val result = client.prfForUnlock(rpId, slots)
            val slot =
                slots.firstOrNull { it.credentialId == result.credentialId }
                    ?: throw PasskeyException(PasskeyError.NO_SLOT, "選んだパスキーのスロットがありません")
            try {
                secretStorage.unlockWithPasskey(result.credentialId, result.prfOutput).use { unlocked ->
                    return SetupSession.fromUnlocked(unlocked) to slot.slotId
                }
            } finally {
                result.prfOutput.fill(0)
            }
        }
    }

    private fun mapReauth(failure: Throwable): Throwable =
        if (isReauthRequired(failure)) E2eeSetupException.ReauthRequired() else failure

    companion object {
        /** 1 回に送るセッションの数（サーバーの上限）。 */
        const val BACKUP_UPLOAD_BATCH = 1000
    }
}

/** パスキーで選べるスロットの一覧（テストと画面で使う）。 */
fun passkeySlots(userId: String, storage: JsonObject): List<PasskeySlot> =
    parseSecretStorage(userId, storage.toString())?.use { it.passkeySlots() }.orEmpty()
