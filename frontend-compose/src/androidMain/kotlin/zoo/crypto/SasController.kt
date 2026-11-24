package zoo.crypto

import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put
import zoo.crypto.ffi.AcceptedMasterKey
import zoo.crypto.ffi.SasPhase
import zoo.crypto.ffi.SasVerification
import zoo.crypto.ffi.SasView
import zoo.crypto.ffi.VerificationMessage
import zoo.crypto.ffi.VerificationOutgoing
import zoo.crypto.ffi.VerifiedDeviceKeys
import zoo.crypto.ffi.ZooCryptoException

/** 他のユーザーの検証（#368）が使う E2EE の API（Web の `megolmApi.ts` と `revocationApi.ts` の一部）。 */
interface SasApi {
    /** `POST /api/e2ee/keys/query`。`raw`（応答そのもの）を使う。 */
    suspend fun queryKeys(userIds: List<String>): KeysQuery

    /** `POST /api/e2ee/keys/claim`。応答の `one_time_keys`（ユーザー、端末、鍵 ID ごとの鍵）を返す。 */
    suspend fun claimKeys(oneTimeKeys: JsonObject): JsonObject

    /**
     * `POST /api/e2ee/to_device`（`type` は `m.room.encrypted`、リクエストごとに別の `txn_id`）。キューの上限などで
     * 入れられなかった宛先（`undelivered` の `user_id` と `device_id`）を返す。
     */
    suspend fun sendToDevice(deviceId: String, messages: JsonObject): List<Pair<String, String>>

    /**
     * `txnId` を指定した `POST /api/e2ee/to_device`（宛先は 100 端末まで）。応答が分からなかった送信を同じ `txn_id` と
     * 内容で送り直すため（サーバーは同じ送信端末・`txn_id`・宛先を 1 件にまとめる）。入れられなかった宛先を返す。
     */
    suspend fun sendToDevice(deviceId: String, txnId: String, messages: JsonObject): List<Pair<String, String>>

    /** `POST /api/e2ee/tofu_pins`。`replace: true` には直前の再認証が要る。 */
    suspend fun uploadPins(pins: JsonArray, replace: Boolean, reauth: String?)

    /** 他のユーザーのマスター鍵への署名を含む `POST /api/e2ee/cross_signing/signatures`（直前の再認証が要る）。 */
    suspend fun uploadSignaturesWithReauth(signatures: JsonArray, reauth: String?)
}

/** 検証の処理の誤り。`code` は [NEEDS_UNLOCK] か [UNKNOWN]（Web の `SasError`）。 */
class SasException(val code: String, message: String) : Exception(message) {
    companion object {
        /** 解錠していないので、検証を求められない・受け入れられない。 */
        const val NEEDS_UNLOCK = "SasNeedsUnlock"

        /** その `transaction_id` の検証がない（閉じたか、アプリを開き直した）。 */
        const val UNKNOWN = "SasUnknown"
    }
}

/** 検証の今の状態と、送るメッセージ。 */
class SasUpdate(val view: SasView, val outgoing: List<VerificationOutgoing>)

/** 検証が終わったあと、相手のマスター鍵への署名とピンの保存の状態（Web の `SasSaveState`）。 */
enum class SasSaveState {
    /** まだ（検証が終わっていない）。 */
    IDLE,

    /** 保存している（再認証を待っているあいだも含む）。 */
    SAVING,
    SAVED,

    /** 再認証できなかった、または古くなった。[SasController.retrySave] で再認証し直して保存する。 */
    REAUTH,

    /** まだ再認証していない（QR を読み取った側）。[SasController.retrySave] で再認証して保存する。 */
    LOGIN,
    FAILED,
}

/** 画面に出す誤り（Web の `SasErrorCode`）。 */
enum class SasErrorCode {
    /** 解錠していないので、受け入れられない。 */
    NEEDS_UNLOCK,

    /** 相手に送れなかった。 */
    SEND_FAILED,

    /** 相手からのメッセージを処理するための鍵を取れなかった（あとで試し直す）。 */
    RECEIVE_FAILED,
    FAILED,
}

/** 読み取った QR の結果。[RETRY] は、取りやめずに読み直す（別の QR、形が正しくない）。 */
enum class QrScanResult { ACCEPTED, RETRY }

/** 1 つの検証の表示（Web の `SasEntry`）。 */
data class SasEntry(
    val view: SasView,
    /** 処理か送信の途中。ボタンを押せなくする。 */
    val busy: Boolean = false,
    val save: SasSaveState = SasSaveState.IDLE,
    val error: SasErrorCode? = null,
) {
    /** 検証済みか、取りやめた。 */
    val finished: Boolean get() = view.finished
}

/** 検証済みか、取りやめた。 */
val SasView.finished: Boolean get() = phase == SasPhase.DONE || phase == SasPhase.CANCELLED

/** 期限切れを確かめ、処理できなかったメッセージを試し直す間隔（Web の `SAS_EXPIRE_INTERVAL_MS`）。 */
val SAS_EXPIRE_INTERVAL: Duration = 5.seconds

/** 検証の期限（10 分）。これより古い未処理のメッセージは捨てる。 */
private const val SAS_TIMEOUT_MS = 10 * 60 * 1000L

/** 1 回の to-device の送信に入れる宛先の端末の上限（サーバーの上限）。 */
internal const val TO_DEVICE_BATCH = 100

private const val REQUEST_TYPE = "m.key.verification.request"

/**
 * 処理に送信ユーザーの `keys/query` が要るメッセージ（送信端末が相手のマスター鍵からたどれるかを確かめる、
 * `SasVerification.fromRequest` と `ready`）。ほかのメッセージは、相手の端末が決まったあとなので鍵を使わない。
 */
private val NEEDS_KEYS = setOf(REQUEST_TYPE, "m.key.verification.ready")

private fun transactionId(message: VerificationMessage): String? =
    try {
        ((CryptoStore.json.parseToJsonElement(message.content) as? JsonObject)?.get("transaction_id") as? JsonPrimitive)
            ?.takeIf { it.isString }?.content
    } catch (_: Exception) {
        null
    }

/**
 * 検証の一覧（Web の `store/verification.ts` の `SasVerifications`）。vodozemac の SAS の状態は保存できないので、
 * 検証はメモリーにだけ置く。アプリを開き直したら続けられない（相手には、10 分で `m.timeout` の取りやめになる）。
 *
 * 鍵を使う処理（求める、要求から作る、署名する）は暗号ストアに任せる。ロックはこのオブジェクト、暗号ストアの順に取る。
 */
internal class SasVerifications {
    private val active = mutableMapOf<String, SasVerification>()

    /** `targetUserId` に検証を求める。`keysQuery` は相手の `keys/query` の応答。 */
    @Synchronized
    fun request(store: CryptoStore, targetUserId: String, keysQuery: String, now: Long): SasUpdate {
        val request = store.requestVerification(keysQuery, targetUserId, now)
        val view = request.verification.view()
        active.put(view.transactionId, request.verification)?.close()
        return SasUpdate(view, request.outgoing)
    }

    /**
     * 届いたメッセージで検証を進める。`keysQuery` は送信ユーザーの `keys/query` の応答。知らない `transaction_id` の
     * メッセージ（要求を除く）と、受け付けない要求（古い、たどれない端末など）は捨てて `null` を返す。
     */
    @Synchronized
    fun receive(store: CryptoStore, message: VerificationMessage, keysQuery: String, now: Long): SasUpdate? {
        val txnId = transactionId(message) ?: return null
        val current = active[txnId]
        if (current == null) {
            if (message.eventType != REQUEST_TYPE) return null
            val verification =
                try {
                    store.verificationFromRequest(message, keysQuery, now)
                } catch (_: ZooCryptoException) {
                    return null
                }
            active[txnId] = verification
            return SasUpdate(verification.view(), emptyList())
        }
        val outgoing = current.receive(message, keysQuery)
        return SasUpdate(current.view(), outgoing)
    }

    @Synchronized
    fun accept(store: CryptoStore, txnId: String): SasUpdate {
        if (!store.hasOwnKeys || store.secretsRevision == null) throw SasException(SasException.NEEDS_UNLOCK, "解錠していません")
        return step(get(txnId)) { it.accept() }
    }

    @Synchronized
    fun confirm(txnId: String): SasUpdate = step(get(txnId)) { it.confirm() }

    @Synchronized
    fun mismatch(txnId: String): SasUpdate = step(get(txnId)) { it.mismatch() }

    /** 絵文字で検証することを選んだ（SAS の `start` を送る）。 */
    @Synchronized
    fun startSas(txnId: String): SasUpdate = step(get(txnId)) { it.startSas() }

    /**
     * 相手の QR を読み取った。形が正しくない、別の検証の QR、読み取れる段階でないときは `null` を返す（取りやめずに
     * 読み直す）。鍵が違えば取りやめる（`view.phase` が `CANCELLED`）。
     */
    @Synchronized
    fun scanQr(txnId: String, bytes: ByteArray): SasUpdate? {
        val verification = get(txnId)
        val outgoing =
            try {
                verification.scanQrCode(bytes)
            } catch (_: ZooCryptoException.InvalidPayload) {
                return null
            } catch (_: ZooCryptoException.Verification) {
                return null
            }
        return SasUpdate(verification.view(), outgoing)
    }

    /** 相手がこの端末の QR を読み取ったことを、利用者が確かめた（`done` を送る）。 */
    @Synchronized
    fun confirmQr(txnId: String): SasUpdate = step(get(txnId)) { it.confirmQrScanned() }

    @Synchronized
    fun cancel(txnId: String): SasUpdate? = active[txnId]?.let { verification -> step(verification) { it.cancel() } }

    /** 10 分を過ぎた検証を取りやめる。変わった検証だけを返す。 */
    @Synchronized
    fun expire(now: Long): List<SasUpdate> =
        active.values.mapNotNull { verification ->
            val outgoing = verification.expire(now.toULong())
            if (outgoing.isEmpty()) null else SasUpdate(verification.view(), outgoing)
        }

    /** 画面を閉じた検証を忘れる。 */
    @Synchronized
    fun forget(txnId: String) {
        active.remove(txnId)?.close()
    }

    /**
     * 検証できた相手のマスター鍵に、自分のユーザー署名鍵で署名する。記録は登録待ちで保存し、`replace: true` の
     * ピンと署名を返す（保存できたら [CryptoStore.confirmAcceptedMasterKey]）。
     */
    @Synchronized
    fun sign(store: CryptoStore, txnId: String, masterKey: String): AcceptedMasterKey =
        store.signVerifiedMasterKey(get(txnId), masterKey)

    @Synchronized
    fun close() {
        active.values.forEach { it.close() }
        active.clear()
    }

    private fun get(txnId: String): SasVerification =
        active[txnId] ?: throw SasException(SasException.UNKNOWN, "検証がありません")

    /** 検証を進め、進めたあとの表示と送るメッセージを返す。 */
    private inline fun step(verification: SasVerification, action: (SasVerification) -> List<VerificationOutgoing>): SasUpdate {
        val outgoing = action(verification)
        return SasUpdate(verification.view(), outgoing)
    }
}

/**
 * 他のユーザーの絵文字 SAS 検証（#368）と QR コードによる検証を進める（Web の `sasController.ts`）。
 *
 * - 状態機械と鍵は [SasVerifications] と暗号ストアにあり、ここは届いたメッセージを渡し、返ったメッセージを Olm で
 *   暗号化して to-device で送る。処理と送信は 1 つずつ順に行う（同じ相手への `mac` と `done` の順を守るため）。
 * - 双方が確かめ終わったら（`done`）、相手のマスター鍵に署名して、`replace: true` のピンと署名を保存する（直前の
 *   再認証が要る。再認証は「一致する」を押したときに始め、[confirm] に渡す）。保存できたら記録の登録待ちを外す。
 * - 検証があるあいだ（終わっていない検証か、処理を待つメッセージがあるあいだ）だけ、[expireInterval] ごとに期限切れを
 *   確かめ、処理できなかったメッセージを試し直す。
 *
 * 画面は [entries] を見て、[saved] で検証済みの表示を読み直す。
 */
class SasController(
    private val api: SasApi,
    /** 登録を終えた暗号ストア（[E2eeDeviceManager.readyStore]）。 */
    private val ready: StateFlow<CryptoStore?>,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val now: () -> Long = System::currentTimeMillis,
    /** 期限切れを確かめる間隔。テストでは 0 にして止める。 */
    private val expireInterval: Duration = SAS_EXPIRE_INTERVAL,
) {
    private class Inbound(val message: VerificationMessage, val receivedAt: Long)

    /**
     * 1 つの端末あての、送れていない検証のメッセージ（`message` は送る順の番号。同じメッセージの宛先は同じ番号）。
     * [device] はこれが持ち、送れたか捨てたときに閉じる。
     */
    private class PendingSend(
        val device: VerifiedDeviceKeys,
        val eventType: String,
        val content: String,
        val message: Long,
        val queuedAt: Long,
    ) {
        val target: Pair<String, String> = device.userId() to device.deviceId()

        /**
         * 一度送った（応答がなかったか、`undelivered` だった）ときの `txn_id` と暗号文。送り直すときは同じものを送る
         * （サーバーは同じ送信端末・`txn_id`・宛先を 1 件にまとめるので、届いていたら二重に届かない）。
         */
        var txnId: String? = null
        var encrypted: JsonElement? = null
    }

    private val verifications = SasVerifications()
    private val queue = Mutex()

    /** [entryMap]、[inbox]、[unsent]、[reauths]、[ticker]、[entriesFlow] への公開を守る。 */
    private val lock = Any()
    private val entryMap = LinkedHashMap<String, SasEntry>()

    /**
     * 届いたが、まだ処理していないメッセージ（届いた順）。Olm のメッセージは 2 回は復号できないので、`keys/query` が
     * 一時的に失敗しても捨てずに残し、次の確認で試し直す。
     */
    private val inbox = mutableListOf<Inbound>()

    /**
     * 送れなかった検証のメッセージ（`transaction_id` ごとに、送る順）。届いたメッセージは送り直さず、届かなかった端末に、
     * 届かなかったメッセージから順に、期限の確認のたびに送り直す（送れたメッセージより後のものも、順を守るため待つ）。
     */
    private val unsent = mutableMapOf<String, List<PendingSend>>()
    private var messageSeq = 0L

    /** 「一致する」を押したときに始めた再認証（検証が終わったら、その ID トークンで保存する）。 */
    private val reauths = mutableMapOf<String, Deferred<String?>>()
    private var ticker: Job? = null

    @Volatile
    private var disposed = false

    private val entriesFlow = MutableStateFlow<List<SasEntry>>(emptyList())
    private val savedFlow = MutableSharedFlow<String>(extraBufferCapacity = 16)

    /** 今の検証（始めた順）。画面はこれを見る。 */
    val entries: StateFlow<List<SasEntry>> get() = entriesFlow

    /** 相手のマスター鍵への署名を保存できた相手の `user_id`（検証済みの表示を読み直す）。 */
    val saved: SharedFlow<String> get() = savedFlow

    /** `userId` に検証を求める。`transaction_id` を返す。解錠していなければ [SasException]（[SasException.NEEDS_UNLOCK]）。 */
    suspend fun request(userId: String): String =
        serial {
            val keysQuery = rawKeys(listOf(userId))
            val store = store()
            val update = withContext(io) { verifications.request(store, userId, keysQuery.toString(), now()) }
            apply(update)
            update.view.transactionId
        }

    /**
     * to-device で届いた検証のメッセージ（[CryptoStore.processToDevice] の `verifications`）。届いた順に受け取り、
     * 処理は裏で行う（返す [Job] で終わりを待てる）。
     */
    fun handleMessages(messages: List<VerificationMessage>): Job {
        val receivedAt = now()
        synchronized(lock) {
            if (disposed) {
                // 止めたあとに届いた分は、受け取らずに捨てる。
                messages.forEach { it.destroy() }
                return Job().apply { complete() }
            }
            messages.forEach { inbox += Inbound(it, receivedAt) }
        }
        ensureTicker()
        return scope.launch { drain() }
    }

    suspend fun accept(txnId: String) = act(txnId) { verifications.accept(store(), txnId) }

    /**
     * 絵文字が一致すると答えた。`reauth` は、このときに始めた再認証（ID トークン）。検証が終わったら、その ID トークンで
     * 署名とピンを保存する。
     */
    suspend fun confirm(txnId: String, reauth: Deferred<String?>) {
        synchronized(lock) { reauths[txnId] = reauth }
        act(txnId) { verifications.confirm(txnId) }
    }

    /** 絵文字で検証することを選んだ（QR を読み取らずに、絵文字 SAS を始める）。 */
    suspend fun startSas(txnId: String) = act(txnId) { verifications.startSas(txnId) }

    /**
     * 相手の QR を読み取った。別の QR や形の正しくないものは [QrScanResult.RETRY]（取りやめずに読み直す）。鍵が違えば
     * 取りやめになり、[QrScanResult.ACCEPTED] を返す（結果は `view.phase` に出る）。
     */
    suspend fun scanQr(txnId: String, bytes: ByteArray): QrScanResult =
        serial {
            patch(txnId) { it.copy(busy = true, error = null) }
            try {
                val update = withContext(io) { verifications.scanQr(txnId, bytes) } ?: return@serial QrScanResult.RETRY
                apply(update)
                QrScanResult.ACCEPTED
            } catch (failure: CancellationException) {
                throw failure
            } catch (failure: Exception) {
                patch(txnId) { it.copy(error = errorCode(failure)) }
                QrScanResult.ACCEPTED
            } finally {
                patch(txnId) { it.copy(busy = false) }
            }
        }

    /**
     * 相手がこの端末の QR を読み取ったと、利用者が答えた。`reauth` は、このときに始めた再認証（ID トークン）。その
     * ID トークンで署名とピンを保存する。
     */
    suspend fun confirmQr(txnId: String, reauth: Deferred<String?>) {
        synchronized(lock) { reauths[txnId] = reauth }
        act(txnId) { verifications.confirmQr(txnId) }
    }

    suspend fun mismatch(txnId: String) = act(txnId) { verifications.mismatch(txnId) }

    suspend fun cancel(txnId: String) = act(txnId) { verifications.cancel(txnId) }

    /** 画面を閉じる。終わっていなければ取りやめてから忘れる。 */
    suspend fun dismiss(txnId: String) {
        val entry = synchronized(lock) { entryMap[txnId] }
        if (entry != null && !entry.finished) cancel(txnId)
        serial {
            verifications.forget(txnId)
            val dropped = synchronized(lock) {
                entryMap.remove(txnId)
                reauths.remove(txnId)
                unsent.remove(txnId).orEmpty()
            }
            dropped.forEach { it.device.destroy() }
            publish()
        }
    }

    /** 再認証し直して、署名とピンを保存し直す。 */
    suspend fun retrySave(txnId: String, reauth: Deferred<String?>) {
        synchronized(lock) { reauths[txnId] = reauth }
        save(txnId)
    }

    /** 10 分を過ぎた検証を取りやめる（`m.timeout`）。処理を待つメッセージがあれば、先に試し直す。 */
    suspend fun expire() {
        if (synchronized(lock) { inbox.isNotEmpty() }) drain()
        if (synchronized(lock) { unsent.isNotEmpty() }) {
            serial {
                for (txnId in synchronized(lock) { unsent.keys.toList() }) {
                    val retry = synchronized(lock) { unsent.remove(txnId).orEmpty() }
                    // 期限を過ぎたものは捨てる。捨てた端末あての後のメッセージも、順を守れないので捨てる（`mac` を捨てて
                    // `done` だけを送らない）。
                    val dead = mutableSetOf<Pair<String, String>>()
                    val live = mutableListOf<PendingSend>()
                    for (item in retry) {
                        if (item.target in dead || now() - item.queuedAt > SAS_TIMEOUT_MS) {
                            dead += item.target
                            item.device.destroy()
                        } else {
                            live += item
                        }
                    }
                    if (live.isNotEmpty()) deliver(txnId, live)
                    else patch(txnId) { if (it.error == SasErrorCode.SEND_FAILED) it.copy(error = null) else it }
                }
            }
        }
        // 終わったのに保存を始めていない検証（保存を始める前に処理が取り消された）は、ここで始める。
        val idle = synchronized(lock) {
            entryMap.values.filter { it.view.phase == SasPhase.DONE && it.save == SasSaveState.IDLE }.map { it.view.transactionId }
        }
        idle.forEach { scope.launch { save(it) } }
        if (synchronized(lock) { entryMap.isEmpty() }) return
        serial {
            val updates = withContext(io) { verifications.expire(now()) }
            for (update in updates) apply(update)
        }
    }

    /** 止める（ログアウトや画面のセッションの終わり）。検証はすべて忘れる。 */
    fun dispose() {
        disposed = true
        synchronized(lock) {
            ticker?.cancel()
            ticker = null
            inbox.forEach { it.message.destroy() }
            inbox.clear()
            unsent.values.forEach { items -> items.forEach { it.device.destroy() } }
            unsent.clear()
        }
        verifications.close()
    }

    /**
     * 届いたメッセージを、届いた順に処理する。`keys/query` を取れなければ、その検証（`transaction_id`）のメッセージ
     * だけを、順を変えずに残す。ほかの検証のメッセージは処理を続ける（鍵を取れない要求が、比べている最中の検証を
     * 止めないように）。残したものは、期限の確認のたびに試し直す。
     */
    private suspend fun drain() =
        serial {
            // 処理する分を取り出す。待つあいだに届いたメッセージは inbox に入るので、最後に、残した分（と、取り消されて
            // 処理しなかった分）をその前に戻す（同じ検証のメッセージの届いた順を守る）。止めたあとなら、戻さずに捨てる。
            val pending = synchronized(lock) { ArrayDeque(inbox).also { inbox.clear() } }
            val remaining = mutableListOf<Inbound>()
            try {
                drainBatch(pending, remaining)
            } finally {
                val leftover = remaining + pending
                synchronized(lock) {
                    if (disposed) leftover.forEach { it.message.destroy() } else inbox.addAll(0, leftover)
                }
            }
        }

    /**
     * 取り出した分を処理する。送信ユーザーの `keys/query` は、この回の中では使い回し、失敗した送信ユーザーは、この回
     * では取り直さない。残すメッセージは `remaining` に届いた順に入れる。処理を終えた（捨てたか残した）ものから
     * `pending` の先頭を外す（取り消されたら、`pending` に残った分を呼び出し元が戻すか捨てる）。
     */
    private suspend fun drainBatch(pending: ArrayDeque<Inbound>, remaining: MutableList<Inbound>) {
        val keys = mutableMapOf<String, String>()
        val failedSenders = mutableSetOf<String>()
        /** この回で `keys/query` を取れず、メッセージを残した検証。 */
        val held = mutableSetOf<String>()
        while (pending.isNotEmpty()) {
            val item = pending.first()
            val message = item.message
            val txnId = transactionId(message)
            if (txnId == null || now() - item.receivedAt > SAS_TIMEOUT_MS) {
                pending.removeFirst()
                message.destroy()
                continue
            }
            // 同じ検証の前のメッセージを残したなら、順を守るため、これも残す。
            if (txnId in held) {
                remaining += pending.removeFirst()
                continue
            }
            var keysQuery = "{}"
            if (message.eventType in NEEDS_KEYS) {
                val sender = message.senderUserId
                var cached = keys[sender]
                if (cached == null && sender !in failedSenders) {
                    try {
                        cached = rawKeys(listOf(sender)).toString().also { keys[sender] = it }
                    } catch (failure: CancellationException) {
                        currentCoroutineContext().ensureActive()
                        failedSenders += sender
                    } catch (_: Exception) {
                        failedSenders += sender
                    }
                }
                if (cached == null) {
                    held += txnId
                    remaining += pending.removeFirst()
                    patch(txnId) { it.copy(error = SasErrorCode.RECEIVE_FAILED) }
                    continue
                }
                keysQuery = cached
            }
            // 暗号ストアを使えない（Shape のつなぎ直しのあいだ）なら、鍵を取れなかったときと同じく、この検証の
            // メッセージを届いた順のまま残す（捨てると、Olm のメッセージは 2 回は復号できないので完了できない）。
            val store = ready.value
            if (store == null) {
                held += txnId
                remaining += pending.removeFirst()
                continue
            }
            val taken = pending.removeFirst()
            // IO の処理を始めたか（始める前に取り消されたら、メッセージを pending の先頭に戻す）。
            var started = false
            // 捨てずに残すか（試し直す）。
            var keep = false
            try {
                // 受け付けた結果は、戻る前に取り消されても登録する（検証の状態は進んでいる）。送信は登録のあとに行う。
                var update: SasUpdate? = null
                try {
                    withContext(io) {
                        started = true
                        update = verifications.receive(store, message, keysQuery, now())
                    }
                } finally {
                    update?.let { register(it) }
                }
                update?.let { received ->
                    val id = received.view.transactionId
                    patch(id) { if (it.error == SasErrorCode.RECEIVE_FAILED) it.copy(error = null) else it }
                    flush(id)
                }
            } catch (failure: CancellationException) {
                if (!started) {
                    pending.addFirst(taken)
                    keep = true
                }
                throw failure
            } catch (_: Exception) {
                if (store.isClosed || ready.value !== store) {
                    // 取ったあとで暗号ストアが閉じられた（つなぎ直し）。受け付けなかったのではないので、この検証の
                    // メッセージを届いた順のまま残し、次の確認で試し直す。
                    held += txnId
                    remaining += taken
                    keep = true
                }
                // それ以外の、受け付けないメッセージは、試し直しても通らない。捨てて続きを処理する。
            } finally {
                if (!keep) message.destroy()
            }
        }
    }

    private suspend fun act(txnId: String, task: () -> SasUpdate?) {
        serial {
            patch(txnId) { it.copy(busy = true, error = null) }
            try {
                var update: SasUpdate? = null
                try {
                    withContext(io) { update = task() }
                } finally {
                    update?.let { register(it) }
                }
                update?.let { flush(it.view.transactionId) }
            } catch (failure: CancellationException) {
                throw failure
            } catch (failure: Exception) {
                patch(txnId) { it.copy(error = errorCode(failure)) }
            } finally {
                patch(txnId) { it.copy(busy = false) }
            }
        }
    }

    /** 状態を出し、送るメッセージを送る。検証が終わったら保存を始める。 */
    private suspend fun apply(update: SasUpdate) {
        register(update)
        flush(update.view.transactionId)
    }

    /**
     * 進めた検証の表示を出し、送るメッセージを送信待ち（[unsent] の後ろ。前に送れなかった分の後）に入れる。中断しない
     * （取り消されても、進めた状態と送る内容を失わないため）。宛先の端末は PendingSend が持つ。
     */
    private fun register(update: SasUpdate) {
        val txnId = update.view.transactionId
        val dropped = synchronized(lock) {
            val current = entryMap[txnId]
            entryMap[txnId] = current?.copy(view = update.view) ?: SasEntry(update.view)
            val fresh = update.outgoing.flatMap { out ->
                val message = messageSeq++
                out.devices.map { PendingSend(it, out.eventType, out.content, message, now()) }
            }
            if (disposed) {
                fresh
            } else {
                if (fresh.isNotEmpty()) unsent[txnId] = unsent[txnId].orEmpty() + fresh
                emptyList()
            }
        }
        dropped.forEach { it.device.destroy() }
        publish()
    }

    /** 送信待ちを送り（前に送れなかった分から順に）、検証が終わっていたら保存を始める。 */
    private suspend fun flush(txnId: String) {
        val items = synchronized(lock) { unsent.remove(txnId).orEmpty() }
        if (items.isNotEmpty()) deliver(txnId, items)
        val entry = synchronized(lock) { entryMap[txnId] }
        if (entry != null && entry.view.phase == SasPhase.DONE && entry.save == SasSaveState.IDLE) {
            // 再認証を待つので、処理の列の外で保存する。
            scope.launch { save(txnId) }
        }
    }

    /**
     * `items` を送り、送れなかった分を [unsent] に残す（次の期限の確認で送り直す）。送れた分と、残せない分（検証を
     * 忘れた、止めた）の端末は閉じる。残した分があれば、画面に [SasErrorCode.SEND_FAILED] を出す。送っている途中で
     * 取り消されたら、届いたと分かった分だけを外し、残りを [unsent] に戻す（止めたあとなら閉じる）。
     */
    private suspend fun deliver(txnId: String, items: List<PendingSend>) {
        val delivered = mutableSetOf<PendingSend>()
        val undelivered =
            try {
                send(items, delivered)
            } catch (failure: CancellationException) {
                keepUnsent(txnId, items.filter { it !in delivered }, items)
                throw failure
            }
        val kept = keepUnsent(txnId, undelivered, items)
        patch(txnId) {
            when {
                kept -> it.copy(error = SasErrorCode.SEND_FAILED)
                it.error == SasErrorCode.SEND_FAILED -> it.copy(error = null)
                else -> it
            }
        }
        if (kept) ensureTicker()
    }

    /** `undelivered` を [unsent] の先頭に戻し（戻せなければ閉じる）、`items` の残りの端末を閉じる。戻したかを返す。 */
    private fun keepUnsent(txnId: String, undelivered: List<PendingSend>, items: List<PendingSend>): Boolean {
        val kept = synchronized(lock) {
            if (undelivered.isNotEmpty() && !disposed && entryMap.containsKey(txnId)) {
                unsent[txnId] = undelivered + unsent[txnId].orEmpty()
                true
            } else {
                false
            }
        }
        val keep = if (kept) undelivered.toSet() else emptySet()
        items.filter { it !in keep }.forEach { it.device.destroy() }
        return kept
    }

    /**
     * 送るメッセージを、宛先の端末ごとに Olm で暗号化して送る。送信用の Olm セッションは鍵の配布と同じものを使い
     * 回し、ない端末は先に `keys/claim` で払い出す。1 つの要求には 1 つの端末あてに 1 件しか入れられないので、
     * メッセージごとに分け、この順に送る。一度送ったものは、同じ `txn_id` と暗号文で送り直す。送れなかった分を、送る
     * 順に返す（ある端末に送れなかったら、その端末あての後のメッセージは送らずに返す。同じ端末あての順を守るため）。
     * 届いたと分かった分は、そのつど `delivered` に入れる（取り消されたときに、送り直さないため）。
     */
    private suspend fun send(items: List<PendingSend>, delivered: MutableSet<PendingSend>): List<PendingSend> {
        if (items.isEmpty()) return emptyList()
        // 暗号ストアを使えない（Shape のつなぎ直しのあいだ）なら、何も送らずにすべて送り直しに回す。
        val store = ready.value ?: return items
        val claimed =
            try {
                val needs = withContext(io) {
                    items.filter { it.encrypted == null }.distinctBy { it.target }.map { it.device }.filter { store.needsOlm(it) }
                }
                if (needs.isEmpty()) JsonObject(emptyMap())
                else api.claimKeys(buildJsonObject {
                    needs.groupBy { it.userId() }.forEach { (user, devices) ->
                        put(user, buildJsonObject { devices.forEach { put(it.deviceId(), "signed_curve25519") } })
                    }
                })
            } catch (failure: CancellationException) {
                throw failure
            } catch (_: Exception) {
                // 払い出せなくても、暗号化済みのものと、今の Olm セッションで暗号化できるものは送る（セッションのない
                // 端末は下で暗号化できず、その端末あての後のメッセージも待たせる）。
                JsonObject(emptyMap())
            }
        val failed = mutableSetOf<Pair<String, String>>()
        val undelivered = mutableSetOf<PendingSend>()
        for (group in items.groupBy { it.message }.values) {
            for (item in group) {
                if (item.target in failed) {
                    undelivered += item
                    continue
                }
                if (item.encrypted != null) continue
                val key = ((claimed[item.target.first] as? JsonObject)?.get(item.target.second) as? JsonObject)?.values?.firstOrNull()
                item.encrypted =
                    try {
                        withContext(io) { store.encryptVerification(item.device, item.eventType, item.content, key?.toString()) }
                    } catch (failure: CancellationException) {
                        throw failure
                    } catch (_: Exception) {
                        null
                    }
                if (item.encrypted == null) {
                    // ワンタイム鍵もフォールバック鍵も払い出されなかった端末。
                    failed += item.target
                    undelivered += item
                }
            }
            // 前に送った分は前の `txn_id` ごとに、新しく暗号化した分は新しい `txn_id`（100 端末ずつ）で送る。
            val ready = group.filter { it.target !in failed && it.encrypted != null }
            val requests = ready.filter { it.txnId != null }.groupBy { it.txnId!! }.values.toMutableList()
            ready.filter { it.txnId == null }.chunked(TO_DEVICE_BATCH).forEach { chunk ->
                val id = java.util.UUID.randomUUID().toString()
                chunk.forEach { it.txnId = id }
                requests += chunk
            }
            for (request in requests) {
                val messages = buildJsonObject {
                    request.groupBy { it.target.first }.forEach { (user, targets) ->
                        put(user, buildJsonObject { targets.forEach { put(it.target.second, it.encrypted!!) } })
                    }
                }
                val refused =
                    try {
                        api.sendToDevice(store.deviceId, request.first().txnId!!, messages).toSet()
                    } catch (failure: CancellationException) {
                        throw failure
                    } catch (_: Exception) {
                        // 応答がなければ、届いたか分からない。同じ `txn_id` と暗号文で送り直す（届いていればサーバーが
                        // まとめるので、二重には届かない）。
                        null
                    }
                for (item in request) {
                    if (refused == null || item.target in refused) {
                        failed += item.target
                        undelivered += item
                    } else {
                        delivered += item
                    }
                }
            }
        }
        // 送る順（メッセージの順）にそろえて返す。
        return items.filter { it in undelivered }
    }

    private suspend fun save(txnId: String) {
        // 確かめることと SAVING にすることを 1 つのロックの中で行う（自動の保存と retrySave が重なっても、保存を
        // 1 つだけ始める）。
        val (theirUserId, reauth) =
            synchronized(lock) {
                val entry = entryMap[txnId] ?: return
                // 保存中と保存済みは始めない（LOGIN・REAUTH・FAILED からの再試行は行う）。
                if (entry.view.phase != SasPhase.DONE || entry.save == SasSaveState.SAVING || entry.save == SasSaveState.SAVED) return
                val reauth = reauths[txnId]
                entryMap[txnId] =
                    if (reauth == null) {
                        // QR を読み取った側は、まだ再認証していない。
                        entry.copy(save = if (entry.save == SasSaveState.IDLE) SasSaveState.LOGIN else SasSaveState.REAUTH)
                    } else {
                        entry.copy(save = SasSaveState.SAVING)
                    }
                entry.view.theirUserId to reauth
            }
        publish()
        if (reauth == null) return
        try {
            val token =
                try {
                    reauth.await()
                } catch (failure: CancellationException) {
                    // 再認証が取り消された（この処理が取り消されたのでなければ、再認証できなかったものとする）。
                    currentCoroutineContext().ensureActive()
                    reauthFailed(txnId)
                    return
                } catch (_: Exception) {
                    reauthFailed(txnId)
                    return
                }
            val keysQuery = rawKeys(listOf(theirUserId))
            val masterKey = (keysQuery["master_keys"] as? JsonObject)?.get(theirUserId) ?: error("相手のマスター鍵がありません")
            val store = store()
            val accepted = withContext(io) { verifications.sign(store, txnId, masterKey.toString()) }
            saveAccepted(store, accepted, token)
            patch(txnId) { it.copy(save = SasSaveState.SAVED) }
            savedFlow.tryEmit(theirUserId)
        } catch (failure: CancellationException) {
            throw failure
        } catch (failure: Exception) {
            if (isReauthRequired(failure)) reauthFailed(txnId)
            else patch(txnId) { it.copy(save = SasSaveState.FAILED) }
        }
    }

    private fun reauthFailed(txnId: String) {
        synchronized(lock) { reauths.remove(txnId) }
        patch(txnId) { it.copy(save = SasSaveState.REAUTH) }
    }

    /**
     * `replace: true` のピンとマスター鍵への署名を保存し、保存できたら記録の登録待ちを外す（Web の
     * `saveAcceptedMasterKey`）。
     */
    private suspend fun saveAccepted(store: CryptoStore, accepted: AcceptedMasterKey, reauth: String?) {
        val pin = CryptoStore.json.parseToJsonElement(accepted.pin) as JsonObject
        val target = (pin["target_user_id"] as? JsonPrimitive)?.contentOrNull ?: error("ピンに相手がありません")
        val masterKey = (pin["master_key"] as? JsonPrimitive)?.contentOrNull ?: error("ピンにマスター鍵がありません")
        // サーバーは `type` と `user_id` を自分で付けるので、署名付きのピンから外して送る。
        val body = buildJsonObject {
            put("target_user_id", target)
            put("master_key", masterKey)
            pin["signatures"]?.let { put("signatures", it) }
        }
        api.uploadPins(JsonArray(listOf(body)), replace = true, reauth = reauth)
        val signature = accepted.signature
        api.uploadSignaturesWithReauth(
            JsonArray(listOf(buildJsonObject {
                put("target_user_id", signature.targetUserId)
                put("target_key_id", signature.targetKeyId)
                put("signer_key_id", signature.signerKeyId)
                put("signature", signature.signature)
            })),
            reauth,
        )
        withContext(io) { store.confirmAcceptedMasterKey(target, masterKey) }
    }

    private suspend fun rawKeys(userIds: List<String>): JsonObject =
        api.queryKeys(userIds).raw ?: error("keys/query の応答がありません")

    private fun store(): CryptoStore = ready.value ?: error("暗号ストアの準備ができていません")

    private fun errorCode(failure: Throwable): SasErrorCode =
        if (failure is SasException && failure.code == SasException.NEEDS_UNLOCK) SasErrorCode.NEEDS_UNLOCK else SasErrorCode.FAILED

    private fun patch(txnId: String, change: (SasEntry) -> SasEntry) {
        synchronized(lock) {
            val entry = entryMap[txnId] ?: return
            entryMap[txnId] = change(entry)
        }
        publish()
    }

    private fun publish() {
        if (disposed) return
        // 写しを取ることと公開することを同じロックの中で行う（古い写しで、新しい公開を上書きしないため）。
        synchronized(lock) { entriesFlow.value = entryMap.values.sortedBy { it.view.startedAt } }
        ensureTicker()
    }

    /** 終わっていない検証か、処理を待つメッセージか、送り直すメッセージがあるあいだだけ、期限切れを確かめる。 */
    private fun active(): Boolean = inbox.isNotEmpty() || unsent.isNotEmpty() || entryMap.values.any { !it.finished }

    private fun ensureTicker() {
        if (expireInterval <= Duration.ZERO || disposed) return
        synchronized(lock) {
            if (ticker != null || !active()) return
            ticker = scope.launch {
                while (true) {
                    delay(expireInterval)
                    try {
                        expire()
                    } catch (failure: CancellationException) {
                        throw failure
                    } catch (_: Exception) {
                        // 次の確認で試し直す。
                    }
                    synchronized(lock) {
                        if (!active()) {
                            ticker = null
                            return@launch
                        }
                    }
                }
            }
        }
    }

    private suspend fun <T> serial(task: suspend () -> T): T = queue.withLock { task() }
}
