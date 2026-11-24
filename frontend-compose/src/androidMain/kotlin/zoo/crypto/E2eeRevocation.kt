package zoo.crypto

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import zoo.ApiException
import zoo.crypto.ffi.RecoveryKey
import zoo.crypto.ffi.ResignNote
import zoo.crypto.ffi.Revocation
import zoo.crypto.ffi.RevocationInput
import zoo.crypto.ffi.RevocationScope
import zoo.crypto.ffi.RevocationStage
import zoo.crypto.ffi.RevocationSummary
import zoo.crypto.ffi.SignatureUpload
import zoo.crypto.ffi.UnlockedSecretStorage
import zoo.crypto.ffi.ZooCryptoException
import zoo.crypto.ffi.newPasskeyKeySlot
import zoo.crypto.ffi.newPrfSalt
import zoo.crypto.ffi.newRecoveryKeySlot
import zoo.crypto.ffi.parseSecretStorage

/**
 * 失効の手順で使う API（docs/e2ee.md の「端末とワンタイム鍵」「クロス署名」「Secret Storage」「鍵バックアップ」。
 * #362）。React の `revocationApi.ts` と同じ要求。直前の再認証が要る要求には `reauth` に ID トークンを渡す。
 *
 * [E2eeApi] と [SasApi] と同じ名前のメソッドは、同じ要求（[ZooE2eeApi] では 1 つの実装）。
 */
interface E2eeRevocationApi : E2eeSetupApi {
    /** `GET /api/e2ee/devices`（自分の、削除されていない端末。端末名と最後に使った日時は本人にだけ返る）。 */
    suspend fun listDevices(): JsonArray

    /** `DELETE /api/e2ee/devices/{device_id}`。 */
    suspend fun deleteDevice(deviceId: String)

    /** `GET /api/e2ee/tofu_pins?after=<カーソル>`。`{"pins": [...], "next": <カーソル>|null}`。 */
    suspend fun tofuPins(after: String?): JsonObject

    /** `POST /api/e2ee/keys/query`（`raw` に、`master_keys` と `signatures` を含む応答そのもの）。 */
    suspend fun queryKeys(userIds: List<String>): KeysQuery

    /** `POST /api/e2ee/secret_storage/secrets`（端末だけの失効）。増やしたあとの `secrets_revision` を返す。 */
    suspend fun putSecrets(body: JsonObject, reauth: String?): Long?

    /** `POST /api/e2ee/key_backup/versions`。作った（同じ鍵なら今の）版の番号を返す。 */
    suspend fun createBackupVersionNumber(body: JsonObject, reauth: String?): String?

    /** `GET /api/e2ee/key_backup/versions/{version}`。消した版と、ない版は `null`。 */
    suspend fun backupVersion(version: String): JsonObject?

    /** `DELETE /api/e2ee/key_backup/versions/{version}`。すでに消えていれば（404）何もしない。今の版だけ再認証が要る。 */
    suspend fun deleteBackupVersion(version: String, reauth: String?)

    /** `POST /api/e2ee/tofu_pins`。`replace: true` には直前の再認証が要る。 */
    suspend fun uploadPins(pins: JsonArray, replace: Boolean, reauth: String?)

    /** 他のユーザーのマスター鍵への署名を含む `POST /api/e2ee/cross_signing/signatures`（直前の再認証が要る）。 */
    suspend fun uploadSignaturesWithReauth(signatures: JsonArray, reauth: String?)
}

/** 失効の誤り（Web の `RevocationConflictError` など）。 */
sealed class RevocationException(message: String) : Exception(message) {
    /**
     * 直前の再認証が要る（403 `E2EE_REAUTH_REQUIRED`）。保存した段は残るので、再認証してから
     * [E2eeRevocation.continueRevocation] を呼び直す（同じ鍵で続ける）。
     */
    class ReauthRequired : RevocationException("再認証が必要です")

    /**
     * 別の端末の失効やリセットと入り混じった（409。`code` は `E2EE_SSSS_CONFLICT`、`E2EE_CROSS_SIGNING_CONFLICT`、
     * `E2EE_WRONG_BACKUP_VERSION` など）。[E2eeRevocation.abandon] で記録を捨て、SSSS を開け直してやり直す。
     */
    class Conflict(val code: String) : RevocationException("別の端末の失効と入り混じりました（$code）")

    /**
     * サーバーの鍵が SSSS の鍵と違う（始めるときの不一致、または保存のあとの読み直しの不一致）。記録を捨て、
     * SSSS を開け直してやり直す（端末だけの失効で始められないときは 1〜5 を選ぶ）。
     */
    class ServerKeyMismatch(message: String) : RevocationException(message)

    /** 終わっていない失効があるので、新しい失効を始めない。続けるか、[E2eeRevocation.abandon] で捨ててから始める。 */
    class Pending : RevocationException("終わっていない失効があります")

    /** 実行を始めた（続けようとした・捨てようとした）失効と、暗号ストアの失効が違う。何も変えていない。 */
    class Changed : RevocationException("実行を始めた失効が、別の失効に置き換わりました")

    /** 続ける失効がない（終えたか、捨てた）。あるいは、アプリを開き直したので [E2eeRevocation.resume] から続ける。 */
    class NothingToContinue : RevocationException("続ける失効がありません")

    /** SSSS を開けていない（[E2eeRevocation.openSecretStorage] を先に呼ぶ）。 */
    class SecretStorageNotOpened : RevocationException("SSSS を開けていません")
}

/** 端末一覧の 1 行（Web の `DeviceView`）。 */
data class RevocationDevice(
    val deviceId: String,
    /** 端末名。攻撃者も付けられるので、端末を見分ける根拠にしない（登録日時と最後に使った日時を並べる）。 */
    val displayName: String?,
    val createdAt: String,
    val lastSeenAt: String?,
    val current: Boolean,
    /** 今の自己署名鍵の署名があるか（なければ、解錠し直すまで鍵が届かない）。 */
    val signed: Boolean,
)

/** [E2eeRevocation.resume] の結果（Web の `ResumeResult`）。 */
sealed interface RevocationResume {
    /** 記録した段から続ける（[E2eeRevocation.continueRevocation] を呼ぶ）。 */
    data class Resumed(val summary: RevocationSummary) : RevocationResume

    /**
     * 1〜5 を SSSS の保存の前に止めたので、最初からやり直す。記録は捨てたが、範囲と削除した端末は引き継ぐ
     * （範囲を黙って端末だけに縮めないため）。SSSS を開け直して [E2eeRevocation.begin] を呼ぶ。
     */
    data class Restart(val scope: RevocationScope, val deletedDevices: List<String>) : RevocationResume

    /**
     * 別の端末が SSSS を作り直したので続けられない。記録は残る。「やり直す」では、この失効（[revocationId]）の
     * 記録だけを [E2eeRevocation.abandon] で捨ててから、同じ範囲と端末で始め直す。
     */
    data class Conflict(
        val revocationId: String,
        val scope: RevocationScope,
        val deletedDevices: List<String>,
    ) : RevocationResume
}

/** 署名し直さなかった（または記録のピンで置き換えた）相手と理由（失効の画面の一覧）。 */
data class NotResignedUser(val userId: String, val reasons: List<ResignNote>)

/** 失効の画面に出す、署名し直さなかった相手の一覧（`user_id` の順）。 */
fun RevocationSummary.notResignedUsers(): List<NotResignedUser> =
    notes.entries.filter { it.value.isNotEmpty() }.sortedBy { it.key }.map { NotResignedUser(it.key, it.value) }

/** [E2eeRevocation.continueRevocation] の結果（Web の `RevocationResult`）。 */
data class RevocationResult(
    val summary: RevocationSummary,
    /** 新しい自己署名鍵で署名されていない自分の端末（解錠し直すまで鍵が届かない。見覚えがなければ削除を勧める）。 */
    val unsignedDevices: List<RevocationDevice>,
    /** 新しい版に移せなかった行の数。 */
    val droppedSessions: Long,
)

/**
 * AuthZ で利用者に行ってもらう手順（docs/e2ee.md の「失効の手順」の【決定】(1)〜(5)）。今の AuthZ では
 * クライアントからセッションとトークンを失効できないので、失効とリセットの画面で、始める前に [BEFORE]、
 * 終えたあとに [AFTER] を、[AuthzRevocationUrls] へのリンクと一緒に出す。値は翻訳キー（Web の `AuthzSteps`）。
 */
enum class AuthzRevocationStep(val translationKey: String) {
    /** (1) 再認証から 5 分以内に、ほかのセッションをすべて失効する（`/settings/security`）。 */
    REVOKE_SESSIONS("e2ee.authz.revokeSessions"),

    /** (2) 現在のセッションを残したまま、すべての client のサービス接続を停止する（`/settings/services`）。 */
    DISCONNECT("e2ee.authz.disconnect"),

    /** (4) E2EE の手順の途中の再認証でトークンが発行されうるので、もう一度サービス接続を停止する。 */
    DISCONNECT_AGAIN("e2ee.authz.disconnectAgain"),

    /** (5) 最後に、現在のセッションからサインアウトする。 */
    SIGN_OUT("e2ee.authz.signOut");

    companion object {
        /** 失効を始める前に済ませてもらう手順（済ませたことを確かめてから進める）。 */
        val BEFORE = listOf(REVOKE_SESSIONS, DISCONNECT)

        /** 失効を終えたあとに行ってもらう手順。 */
        val AFTER = listOf(DISCONNECT_AGAIN, SIGN_OUT)
    }
}

/** AuthZ のセッション（`/settings/security`）とサービス接続（`/settings/services`）の画面の URL。 */
data class AuthzRevocationUrls(val security: String, val services: String) {
    companion object {
        /** issuer（例：`https://account.zli.works/api/auth`）と同じ origin の画面（Web の `accountRevocationUrls`）。 */
        fun fromIssuer(issuer: String): AuthzRevocationUrls? =
            try {
                val url = java.net.URI(issuer)
                if (url.scheme == null || url.host == null) {
                    null
                } else {
                    val origin = "${url.scheme}://${url.rawAuthority}"
                    AuthzRevocationUrls("$origin/settings/security", "$origin/settings/services")
                }
            } catch (_: java.net.URISyntaxException) {
                null
            }
    }
}

private val STAGE_ORDER =
    listOf(
        RevocationStage.PREPARED,
        RevocationStage.SECRET_STORAGE_SAVED,
        RevocationStage.CROSS_SIGNING_SAVED,
        RevocationStage.SIGNATURES_SAVED,
        RevocationStage.BACKUP_CREATED,
        RevocationStage.MIGRATED,
        RevocationStage.VERIFIED,
    )

private fun RevocationSummary.atLeast(stage: RevocationStage): Boolean =
    STAGE_ORDER.indexOf(this.stage) >= STAGE_ORDER.indexOf(stage)

private fun parseObject(value: String): JsonObject = CryptoStore.json.parseToJsonElement(value) as JsonObject

private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull

private fun JsonObject.secretsRevision(): Long? = (this["secrets_revision"] as? JsonPrimitive)?.longOrNull

private fun SignatureUpload.toJson(): JsonObject = buildJsonObject {
    put("target_user_id", targetUserId)
    put("target_key_id", targetKeyId)
    put("signer_key_id", signerKeyId)
    put("signature", signature)
}

/** 署名付きのピンから、サーバーが自分で付ける `type` と `user_id` を外す（Web の `uploadPins`）。 */
private fun pinBody(pin: String): JsonObject {
    val parsed = parseObject(pin)
    return buildJsonObject {
        parsed["target_user_id"]?.let { put("target_user_id", it) }
        parsed["master_key"]?.let { put("master_key", it) }
        parsed["signatures"]?.let { put("signatures", it) }
    }
}

/**
 * 失効の手順（docs/e2ee.md の「失効の手順」「失効と検証の実装（#362）」）。React の `revocationController.ts`
 * と `revocationSession.ts` と同じ順で要求を送る。画面はなく、画面（設定の端末のセクション）がここの関数を順に呼ぶ。
 *
 * 1. [openSecretStorage]：パスキーかリカバリーキーで SSSS を開け直す（マスター鍵の秘密鍵は暗号ストアにないため）。
 * 2. [deleteSelectedDevices]：選んだ端末を削除する（端末の削除は失効ではない。続けて 3 以降を行う）。
 * 3. [begin]：署名し直す一覧を集め、新しい鍵を作って暗号ストアに保存する（サーバーにはまだ保存しない）。
 * 4. 1〜5（[RevocationScope.FULL]）なら [newRecoveryKey] で新しいリカバリーキーを作って控えてもらい、
 *    任意で [addPasskey] でこの端末のパスキーのスロットを 1 つ足す。
 * 5. [RevocationSummary.notResignedUsers] と `versionsWithoutKey` を見せて確かめてもらう。
 * 6. [continueRevocation]：再認証の ID トークンで、SSSS、クロス署名鍵、署名とピン、鍵バックアップの版、古い版の移行と
 *    削除、読み直しの確認の順に最後まで保存する。保存が 1 つ済むたびに暗号ストアに記録する。
 *
 * アプリを閉じて止まったら、[status] で記録を読み、[openSecretStorage] のあと [resume] で続ける。AuthZ のセッションと
 * トークンの失効はクライアントから行えないので、画面は [AuthzRevocationStep] と [AuthzRevocationUrls] を始める前と
 * 終えたあとに出す。
 *
 * マスター鍵の秘密鍵と SSSS 鍵は UniFFI の [Revocation] と [UnlockedSecretStorage]（Rust のメモリー）にだけ置く。
 */
class E2eeRevocation(
    private val api: E2eeRevocationApi,
    val userId: String,
    private val store: suspend () -> CryptoStore,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) {
    /** 手順を 1 つずつにする（画面の操作が重なっても、同じ失効に保存を重ねない）。 */
    private val lock = Mutex()

    /** 進めている失効（[begin] か [resume] で作る）。 */
    private var revocation: Revocation? = null

    /** 2〜5 で作る、新しい SSSS 鍵の鍵スロット（JSON）。 */
    private val slots = mutableListOf<String>()

    /** [openSecretStorage] で開けた SSSS（[begin] か [resume] で使い、閉じる）。 */
    private var opened: UnlockedSecretStorage? = null

    private val summaryFlow = MutableStateFlow<RevocationSummary?>(null)

    /**
     * 終わっていない失効の進み具合（なければ `null`）。[status] で暗号ストアから読み、保存が 1 つ済むたびに更新する。
     * `null` でないあいだは、端末のセクションに「失効を続ける」を出し、ログアウトの前に警告する。
     */
    val summary: StateFlow<RevocationSummary?> get() = summaryFlow

    /** 暗号ストアに保存した失効の進み具合を読む（解錠していなくても読める）。なければ `null`。 */
    suspend fun status(): RevocationSummary? {
        val store = store()
        return withContext(io) { store.revocationSummary() }.also { summaryFlow.value = it }
    }

    /** 終わっていない失効があるか（ログアウトの警告に使う）。 */
    suspend fun isPending(): Boolean {
        val store = store()
        return withContext(io) { store.hasPendingRevocation }
    }

    /**
     * 自分の端末の一覧。今の自己署名鍵の署名があるかは、表示のために署名があるかだけを見る（配布先の判定では、
     * 署名を検証する）。`currentDeviceId` は今の端末（なければ `null`）。
     */
    suspend fun loadDevices(currentDeviceId: String?): List<RevocationDevice> = coroutineScope {
        val devices = async { api.listDevices() }
        val keys = async { api.queryOwnKeys(userId) }
        val rows = devices.await().mapNotNull { it as? JsonObject }
        val ids = rows.mapNotNull { it.text("device_id") }
        val unsigned = unsignedDevices(keys.await(), userId, ids)
        rows.mapNotNull { row ->
            val id = row.text("device_id") ?: return@mapNotNull null
            RevocationDevice(
                deviceId = id,
                displayName = row.text("display_name"),
                createdAt = row.text("created_at").orEmpty(),
                lastSeenAt = row.text("last_seen_at"),
                current = id == currentDeviceId,
                signed = id !in unsigned,
            )
        }
    }

    /**
     * 始める（続ける）前に、パスキーかリカバリーキーで SSSS を開け直す。続きの保存はしない（初回設定の解錠
     * [E2eeSetup.unlock] とは違い、暗号ストアの自分の鍵も変えない）。開けた SSSS は次の [begin] か [resume] で使う。
     */
    suspend fun openSecretStorage(method: UnlockMethod) {
        val storage = api.secretStorage()
        val secretStorage = parseSecretStorage(userId, storage.toString()) ?: throw E2eeSetupException.NoSecretStorage()
        val (unlocked, slotId) =
            secretStorage.use { parsed ->
                when (method) {
                    is UnlockMethod.Recovery ->
                        RecoveryKey.parse(method.recoveryKey).use { recovery ->
                            parsed.unlockWithRecoveryKey(recovery)
                        } to recoverySlotId(storage)
                    is UnlockMethod.Passkey -> {
                        val slots = parsed.passkeySlots()
                        if (slots.isEmpty()) throw PasskeyException(PasskeyError.NO_SLOT, "パスキーのスロットがありません")
                        val result = method.client.prfForUnlock(method.rpId, slots)
                        try {
                            val slot =
                                slots.firstOrNull { it.credentialId == result.credentialId }
                                    ?: throw PasskeyException(PasskeyError.NO_SLOT, "選んだパスキーのスロットがありません")
                            parsed.unlockWithPasskey(result.credentialId, result.prfOutput) to slot.slotId
                        } finally {
                            result.prfOutput.fill(0)
                        }
                    }
                }
            }
        lock.withLock {
            opened?.close()
            opened = unlocked
        }
        slotId?.let { api.markKeySlotUsed(it) }
    }

    private fun recoverySlotId(storage: JsonObject): String? =
        (storage["slots"] as? JsonArray)?.mapNotNull { it as? JsonObject }?.firstOrNull { it.text("kind") == "recovery" }?.text("id")

    /** 失効の手順の 1：選んだ端末を削除する（端末の削除は失効ではない。続けて [begin] 以降を行う）。 */
    suspend fun deleteSelectedDevices(deviceIds: List<String>) {
        for (deviceId in deviceIds) api.deleteDevice(deviceId)
    }

    /**
     * 失効を始める。署名し直す一覧を集め、新しい鍵を作って暗号ストアに保存する（サーバーにはまだ何も保存しない）。
     * [openSecretStorage] で SSSS を開けてあることが前提。
     *
     * - 終わっていない失効があれば [RevocationException.Pending]（黙って捨てない）。
     * - SSSS の鍵とサーバーの鍵が違えば [RevocationException.ServerKeyMismatch]（端末だけの失効は行えない。1〜5 を選ぶ）。
     */
    suspend fun begin(scope: RevocationScope, deletedDevices: List<String>): RevocationSummary =
        lock.withLock {
            val store = store()
            if (withContext(io) { store.hasPendingRevocation }) throw RevocationException.Pending()
            val unlocked = opened ?: throw RevocationException.SecretStorageNotOpened()
            opened = null
            try {
                val (keysQuery, versions, pins) =
                    coroutineScope {
                        val keys = async { api.queryOwnKeys(userId) }
                        val versions = async { api.backupVersions() }
                        val pins = async { listAllPins() }
                        Triple(keys.await(), versions.await(), pins.await())
                    }
                // 署名し直す候補：ピンか記録がある相手（この相手を `keys/query` で問い合わせ、`signatures` と
                // `master_keys` を使う。自分を署名者とする署名を持つ相手は、受け入れでも SAS でもピンを作る）。
                val targets = linkedSetOf<String>()
                pins.mapNotNullTo(targets) { it.text("target_user_id") }
                targets += withContext(io) { store.trustRecordedUsers() }
                targets -= userId
                val targetsQuery = if (targets.isEmpty()) null else api.queryKeys(targets.toList()).raw
                val input =
                    RevocationInput(
                        scope = scope,
                        keysQuery = keysQuery.toString(),
                        versions = versions.toString(),
                        pins = pins.map { it.toString() },
                        targetsQuery = targetsQuery?.toString(),
                        deletedDevices = deletedDevices,
                    )
                val begun =
                    try {
                        withContext(io) { store.beginRevocation(input, unlocked) }
                    } catch (failure: ZooCryptoException.ServerKeyMismatch) {
                        throw RevocationException.ServerKeyMismatch(failure.message.orEmpty())
                    }
                discard()
                revocation = begun
                begun.summary().also { summaryFlow.value = it }
            } finally {
                unlocked.close()
            }
        }

    /**
     * 止まった失効を、[openSecretStorage] で開け直した SSSS で続ける（2〜5 では、SSSS を保存したあとなら新しい
     * SSSS 鍵のスロットで開ける）。`expectedId` を渡したら、暗号ストアの記録がその失効のときだけ続ける。
     */
    suspend fun resume(expectedId: String? = null): RevocationResume =
        lock.withLock {
            val store = store()
            // やり直しになったら記録を捨てるので、その前に範囲と削除した端末を読んでおく。
            val saved = withContext(io) { store.revocationSummary() } ?: throw RevocationException.NothingToContinue()
            if (expectedId != null && saved.revocationId != expectedId) throw RevocationException.Changed()
            val unlocked = opened ?: throw RevocationException.SecretStorageNotOpened()
            opened = null
            try {
                discard()
                val resumed =
                    withContext(io) { store.resumeRevocation(unlocked) } ?: throw RevocationException.NothingToContinue()
                revocation = resumed
                RevocationResume.Resumed(resumed.summary().also { summaryFlow.value = it })
            } catch (_: ZooCryptoException.RevocationRestart) {
                withContext(io) { store.clearRevocation() }
                summaryFlow.value = null
                RevocationResume.Restart(saved.scope, saved.deletedDevices)
            } catch (_: ZooCryptoException.ServerKeyMismatch) {
                summaryFlow.value = saved
                RevocationResume.Conflict(saved.revocationId, saved.scope, saved.deletedDevices)
            } finally {
                unlocked.close()
            }
        }

    /**
     * 2〜5：新しい SSSS 鍵のリカバリーキーを作る。返したリカバリーキーを利用者に控えてもらう。1〜5 では必ず作る
     * （古いリカバリーキーで新しい SSSS 鍵のスロットを作る経路は持たない）。呼び直すと置き換える。
     */
    suspend fun newRecoveryKey(): String =
        lock.withLock {
            val revocation = requireRevocation()
            RecoveryKey.generate().use { recovery ->
                revocation.ssssKey().use { ssss ->
                    val slot = newRecoveryKeySlot(userId, ssss, recovery)
                    slots.removeAll { parseObject(it).text("kind") == "recovery" }
                    slots += slot
                }
                recovery.toDisplay()
            }
        }

    /** 2〜5：新しい SSSS 鍵に、この端末のパスキーのスロットを作る（新しい `prf_salt`）。 */
    suspend fun addPasskey(client: PasskeyClient, rpId: String, label: String?) {
        val salt = newPrfSalt()
        val result = client.prfForNewSlot(rpId, salt)
        try {
            lock.withLock {
                val revocation = requireRevocation()
                val slot =
                    revocation.ssssKey().use { ssss ->
                        val json = parseObject(newPasskeyKeySlot(userId, ssss, result.credentialId, salt, result.prfOutput))
                        if (label == null) json else JsonObject(json + ("label" to JsonPrimitive(label)))
                    }
                slots.removeAll { parseObject(it).text("credential_id") == result.credentialId }
                slots += slot.toString()
            }
        } finally {
            result.prfOutput.fill(0)
        }
    }

    /**
     * 記録した段の続きから、最後まで保存する（SSSS、クロス署名鍵、署名とピン、鍵バックアップの版、古い版の移行と
     * 削除、読み直しの確認）。`reauth` は 5 分以内の再認証の ID トークン（なければ `null`）。
     *
     * - 403 `E2EE_REAUTH_REQUIRED` なら、`reauthenticate`（Android の `platform.reauthenticate`）を 1 回だけ呼んで
     *   得た ID トークンで、同じ鍵のまま続きから送り直す。渡していないか `null` を返したら
     *   [RevocationException.ReauthRequired]（再認証してから、同じ関数を呼び直す）。
     * - 409（別の端末の失効と入り混じった）なら [RevocationException.Conflict]。[abandon] で記録を捨て、SSSS を
     *   開け直して [begin] からやり直す。
     * - 読み直しで一致しなければ [RevocationException.ServerKeyMismatch]。同じくやり直す。
     * - `expectedId`（実行を始めたときの失効）が今の失効と違えば、何も保存せずに [RevocationException.Changed]。
     *
     * 通信の失敗などはそのまま投げる。保存した段は暗号ストアに残るので、同じ関数を呼び直すと続きから送る。
     */
    suspend fun continueRevocation(
        reauth: String?,
        reauthenticate: (suspend () -> String?)? = null,
        expectedId: String? = null,
        onProgress: (RevocationSummary) -> Unit = {},
    ): RevocationResult =
        lock.withLock {
            var token = reauth
            var reauthenticated = false
            while (true) {
                try {
                    return@withLock runRevocation(token, expectedId, onProgress)
                } catch (failure: RevocationException.ReauthRequired) {
                    if (reauthenticated || reauthenticate == null) throw failure
                    reauthenticated = true
                    token = reauthenticate() ?: throw failure
                }
            }
            @Suppress("UNREACHABLE_CODE")
            error("unreachable")
        }

    private suspend fun runRevocation(
        reauth: String?,
        expectedId: String?,
        onProgress: (RevocationSummary) -> Unit,
    ): RevocationResult {
        val revocation = revocation ?: throw RevocationException.NothingToContinue()
        val store = store()
        var summary = revocation.summary()
        if (expectedId != null && summary.revocationId != expectedId) throw RevocationException.Changed()
        // 暗号ストアの記録が別の失効に置き換わっていたら（別の画面で捨てて始め直した）、何も保存しない。
        val stored = withContext(io) { store.revocationSummary() }
        if (stored?.revocationId != summary.revocationId) throw RevocationException.Changed()
        fun progress() {
            summary = revocation.summary()
            summaryFlow.value = summary
            onProgress(summary)
        }
        suspend fun save() = withContext(io) { store.saveRevocation(revocation) }
        var secretsRevision: Long? = null

        if (!summary.atLeast(RevocationStage.SECRET_STORAGE_SAVED)) {
            val body = parseObject(revocation.secretStorageRequest(slots.toList()))
            secretsRevision =
                try {
                    guarded {
                        if (summary.scope == RevocationScope.FULL) api.createSecretStorageKey(body, reauth)
                        else api.putSecrets(body, reauth)
                    }
                } catch (failure: RevocationException.Conflict) {
                    // 応答をなくして送り直したか、別の端末が先に書いた。読み直して、自分の書き込みが済んでいれば続ける。
                    if (failure.code != "E2EE_SSSS_CONFLICT") throw failure
                    val current = api.secretStorage()
                    if (summary.scope != RevocationScope.FULL || current.text("default_key_id") != summary.keyId) throw failure
                    current.secretsRevision()
                }
            revocation.secretStorageSaved(secretsRevision?.toULong())
            save()
            progress()
        }
        if (secretsRevision == null) secretsRevision = api.secretStorage().secretsRevision()

        if (!summary.atLeast(RevocationStage.CROSS_SIGNING_SAVED)) {
            val body = parseObject(revocation.crossSigningRequest())
            guarded { api.uploadCrossSigningKeys(body, reauth) }
            revocation.advance(RevocationStage.CROSS_SIGNING_SAVED)
            // 記録と自分の鍵を新しい鍵にそろえ、進み具合と同じ書き込みで保存する。
            withContext(io) { store.applyRevocation(revocation, secretsRevision) }
            progress()
        }

        if (!summary.atLeast(RevocationStage.SIGNATURES_SAVED)) {
            // 今の端末への自己署名は、相手のマスター鍵への署名と分けて先に送る（相手の都合で止めない）。
            val own = withContext(io) { store.revocationSignatures(revocation) }
            guarded { api.uploadSignaturesWithReauth(JsonArray(listOf(own.device.toJson())), reauth) }
            // 失効を始めてから相手がマスター鍵を作り直していると、その署名はサーバーが 404 で拒む。送る前に今の
            // マスター鍵と照合して外し（注記に回す）、照合と送信のあいだに作り直されたら照合し直す。
            var signatures = own
            var attempt = 0
            while (true) {
                val targets = summary.resignedMasterKeys
                if (targets.isNotEmpty()) {
                    val current = api.queryKeys(targets).raw ?: JsonObject(emptyMap())
                    val masterKeys = current["master_keys"] as? JsonObject ?: JsonObject(emptyMap())
                    if (revocation.dropStaleMasterKeys(masterKeys.toString()).isNotEmpty()) save()
                    progress()
                    signatures = withContext(io) { store.revocationSignatures(revocation) }
                }
                try {
                    if (signatures.masterKeys.isNotEmpty()) {
                        guarded {
                            api.uploadSignaturesWithReauth(JsonArray(signatures.masterKeys.map { it.toJson() }), reauth)
                        }
                    }
                    break
                } catch (failure: ApiException) {
                    if (failure.status != 404 || attempt >= 2) throw failure
                    attempt += 1
                }
            }
            if (signatures.pins.isNotEmpty()) {
                guarded { api.uploadPins(JsonArray(signatures.pins.map(::pinBody)), replace = true, reauth = reauth) }
            }
            revocation.advance(RevocationStage.SIGNATURES_SAVED)
            save()
            progress()
        }

        if (!summary.atLeast(RevocationStage.BACKUP_CREATED)) {
            val body = parseObject(revocation.backupVersionRequest())
            val version = guarded { api.createBackupVersionNumber(body, reauth) } ?: error("版の番号が返りませんでした")
            revocation.backupCreated(version)
            save()
            progress()
        }

        if (!summary.atLeast(RevocationStage.MIGRATED)) {
            val newVersion = summary.newBackupVersion ?: error("新しい版がありません")
            val migrate = revocation.versionsToMigrate(api.backupVersions().toString())
            try {
                // 移行の行も、新しい版の `upload_key` への証明を付けて送る（docs/e2ee.md の「アップロードの証明」）。
                // 証明はトークンだけを持つ者の先取りを防ぐもので、行を送るたびに同じ値を使う。
                var proofResolved = false
                var proof: String? = null
                for (old in migrate) {
                    // 別の端末が先に移行を終えて消した。
                    val rows = listBackupSessions(old.version) ?: continue
                    // 捨てた行の数は、失効の記録に版ごとに残す（古い版を消したあとで止まっても数え直せないため）。
                    val result = revocation.reencryptRows(old.key, rows.map { it.toString() })
                    revocation.recordDropped(old.version, result.dropped)
                    save()
                    if (!proofResolved) {
                        val target = api.backupVersion(newVersion)
                        // フィールドがない版（証明を足す前のサーバー）だけ、証明を付けずに送る。フィールドがあって
                        // 文字列でなければ、送らず古い版も消さずに止める（未証明の行を移したことにしないため）。
                        val uploadKey = target?.get("upload_key")
                        if (uploadKey != null && (uploadKey !is JsonPrimitive || !uploadKey.isString)) {
                            throw IllegalStateException("版 $newVersion の upload_key が鍵の形ではありません")
                        }
                        proof = uploadKey?.content?.let { revocation.uploadProof(it, userId, newVersion) }
                        proofResolved = true
                    }
                    val revision = checkNotNull(secretsRevision) { "secrets_revision がありません" }
                    for (chunk in result.rows.map(::parseObject).chunked(E2eeSetup.BACKUP_UPLOAD_BATCH)) {
                        val body = buildJsonObject { put("sessions", JsonArray(chunk)) }
                        guarded { api.uploadBackupKeys(newVersion, revision, body, proof) }
                    }
                    // 今の版ではないので、再認証は要らない。
                    api.deleteBackupVersion(old.version, null)
                }
            } finally {
                migrate.forEach { it.key.close() }
            }
            revocation.advance(RevocationStage.MIGRATED)
            save()
            progress()
        }

        val (secretStorage, keysQuery, current) =
            coroutineScope {
                val storage = async { api.secretStorage() }
                val keys = async { api.queryOwnKeys(userId) }
                val current = async { api.currentBackupVersion() }
                Triple(storage.await(), keys.await(), current.await())
            }
        val finished = summary
        // 一覧は、記録を消す前に読む。あとで読むと、読めなかったときに、終わった失効を失敗として扱い、記録がないので
        // 続けられなくなる。
        val devices = loadDevices(null)
        try {
            revocation.verify(secretStorage.toString(), keysQuery.toString(), (current ?: JsonObject(emptyMap())).toString())
        } catch (failure: ZooCryptoException) {
            throw RevocationException.ServerKeyMismatch(failure.message.orEmpty())
        }
        withContext(io) { store.clearRevocation() }
        discard()
        summaryFlow.value = null
        return RevocationResult(
            summary = finished.copy(stage = RevocationStage.VERIFIED),
            unsignedDevices = devices.filter { !it.signed },
            droppedSessions = finished.droppedSessions.toLong(),
        )
    }

    /**
     * やり直すために、保存した失効を捨てる。`expectedId` を渡したら、暗号ストアの記録がその失効のときだけ捨てる
     * （違えば何も変えずに [RevocationException.Changed]）。
     */
    suspend fun abandon(expectedId: String? = null) {
        lock.withLock {
            val store = store()
            if (expectedId != null) {
                val stored = withContext(io) { store.revocationSummary() }
                if (stored != null && stored.revocationId != expectedId) throw RevocationException.Changed()
            }
            discard()
            withContext(io) { store.clearRevocation() }
            summaryFlow.value = null
        }
    }

    /**
     * メモリーの秘密（進めている失効、新しいスロット、開けた SSSS）を捨てる（画面を閉じたとき、ログアウト）。
     * 手順の途中（保存の応答を待っている）なら待たずに何もしない（その手順が終われば、続きは暗号ストアの記録から行う）。
     */
    fun dispose() {
        if (!lock.tryLock()) return
        try {
            discard()
            opened?.close()
            opened = null
        } finally {
            lock.unlock()
        }
    }

    private fun discard() {
        revocation?.close()
        revocation = null
        slots.clear()
    }

    private fun requireRevocation(): Revocation = revocation ?: throw RevocationException.NothingToContinue()

    /** `GET /api/e2ee/tofu_pins` を `next` が `null` になるまで読む。 */
    private suspend fun listAllPins(): List<JsonObject> {
        val pins = mutableListOf<JsonObject>()
        var after: String? = null
        do {
            val page = api.tofuPins(after)
            (page["pins"] as? JsonArray)?.mapNotNullTo(pins) { it as? JsonObject }
            after = page.text("next")
        } while (after != null)
        return pins
    }

    /** `GET /api/e2ee/key_backup/keys?version=` を `next` が `null` になるまで読む。消した版は `null`。 */
    private suspend fun listBackupSessions(version: String): List<JsonElement>? {
        val rows = mutableListOf<JsonElement>()
        var after: String? = null
        try {
            do {
                val page = api.backupKeys(version, after)
                (page["sessions"] as? JsonArray)?.let(rows::addAll)
                after = page.text("next")
            } while (after != null)
        } catch (failure: ApiException) {
            if (failure.status == 404) return null
            throw failure
        }
        return rows
    }

    private suspend fun <T> guarded(block: suspend () -> T): T =
        try {
            block()
        } catch (failure: ApiException) {
            if (isReauthRequired(failure)) throw RevocationException.ReauthRequired()
            if (failure.status == 409) throw RevocationException.Conflict(failure.code() ?: "CONFLICT")
            throw failure
        }
}
