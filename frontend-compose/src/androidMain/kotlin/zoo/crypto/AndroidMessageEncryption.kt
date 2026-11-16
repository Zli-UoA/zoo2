package zoo.crypto

import io.ktor.http.encodeURLParameter
import java.util.UUID
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.StateFlow
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*
import zoo.*
import zoo.crypto.ffi.*

/** Web/TUI と同じ recipients・TOFU・Olm/Megolm を使う。暗号文だけを REST/Shape に渡す。 */
class AndroidMessageEncryption(
    private val ready: StateFlow<CryptoStore?>,
    private val api: ZooApi,
    scope: CoroutineScope,
    private val uploadBackup: suspend () -> Unit = {},
) : MessageEncryption {
    constructor(manager: E2eeDeviceManager, api: ZooApi, scope: CoroutineScope) :
        this(manager.readyStore, api, scope, { manager.setup?.uploadKeyBackup(); Unit })
    override val changes = MutableStateFlow(0L)
    private val sending = Mutex()
    private class SendingSession(val owner: CryptoStore, val session: OutboundSession) {
        val shared = mutableSetOf<String>()
    }
    private val outbound = mutableMapOf<String, SendingSession>()
    private val latestEpochs = mutableMapOf<String, ULong>()
    private fun deviceKey(device: VerifiedDeviceKeys) = "${device.userId()}|${device.deviceId()}|${device.curve25519Key()}"
    private fun discard(channel: String) { outbound.remove(channel)?.session?.close() }
    private val backups = Channel<Unit>(Channel.CONFLATED)

    init {
        scope.launch {
            try { awaitCancellation() }
            finally { withContext(NonCancellable) { sending.withLock {
                outbound.values.forEach { it.session.close() }
                outbound.clear()
            } } }
        }
        scope.launch {
            for (trigger in backups) {
                try { uploadBackup() }
                catch (failure: CancellationException) { throw failure }
                catch (_: Exception) { /* 未送信の鍵は次の受信・送信・起動時にも再送する。 */ }
            }
        }
        scope.launch { ready.collectLatest { store ->
            changes.value++
            store?.changes?.collect { changes.value++ }
        } }
    }

    private fun store(): CryptoStore = checkNotNull(ready.value) { "e2ee.deviceNotReady" }
    private suspend fun <T> crypto(block: () -> T): T = withContext(Dispatchers.IO) { block() }
    private suspend fun request(path: String, method: String = "GET", body: JsonObject? = null): JsonObject =
        withRequestTimeout(30.seconds) {
            val response = api.request(path, method, body) as? JsonObject ?: JsonObject(emptyMap())
            response["data"] as? JsonObject ?: response
        }

    private suspend fun pages(channel: String, minimum: ULong): List<JsonObject> {
        repeat(5) {
            val pages = mutableListOf<JsonObject>()
            var epoch: String? = null
            var after: String? = null
            try {
                do {
                    val query = listOfNotNull(epoch?.let { "epoch=$it" }, after?.let { "after=${it.encodeURLParameter()}" })
                        .joinToString("&").let { if (it.isEmpty()) "" else "?$it" }
                    val page = request("/api/e2ee/channels/${channel.encodeURLParameter()}/recipients$query")
                    val current = page.text("membership_epoch").toULong()
                    if (current < minimum || epoch != null && epoch != current.toString()) break
                    epoch = current.toString()
                    pages += page
                    check(pages.size <= 200) { "配布先のページ数が上限を超えました" }
                    val previous = after
                    after = (page["next_after"] as? JsonPrimitive)?.contentOrNull
                    check(after == null || after != previous) { "配布先のページが進みませんでした" }
                } while (after != null)
                if (epoch != null && epoch.toULong() >= minimum && after == null) return pages
            } catch (failure: ApiException) {
                if (failure.code() != "E2EE_STALE_EPOCH") throw failure
            }
        }
        error("e2ee.sendStaleEpoch")
    }

    private suspend fun recipients(store: CryptoStore, channel: String, minimum: ULong): Pair<ULong, List<VerifiedDeviceKeys>> {
        var pages = pages(channel, minimum)
        repeat(5) {
            val distribution = crypto { store.recipients(pages) }
            var handedOff = false
            try {
                if (distribution.newPins.isEmpty()) {
                    // 配れない参加者を黙って除外しない。Web での確認・再検証後に再試行できる。
                    check(distribution.warnings.isEmpty() && distribution.users.values.all { it.state is UserState.Trusted })
                    check(distribution.users.keys.all { user -> user == store.userId || distribution.devices.any { it.userId() == user } })
                    handedOff = true
                    return pages.first().text("membership_epoch").toULong() to distribution.devices
                }
                var conflict = false
                for (pin in distribution.newPins) {
                    val accepted = try {
                        request("/api/e2ee/tofu_pins", "POST", buildJsonObject {
                            put("pins", JsonArray(listOf(json.parseToJsonElement(pin.pin))))
                            put("replace", false)
                        })
                        true
                    } catch (failure: ApiException) {
                        if (failure.code() != "E2EE_PIN_CONFLICT") throw failure
                        conflict = true
                        false
                    }
                    crypto { store.confirmPin(pin, accepted) }
                }
                if (conflict) pages = pages(channel, minimum)
            } finally {
                if (!handedOff) distribution.devices.forEach { it.close() }
            }
        }
        error("TOFU のピンを確定できませんでした")
    }

    private suspend fun share(store: CryptoStore, outbound: OutboundSession, devices: List<VerifiedDeviceKeys>) {
        for (batch in devices.chunked(100)) {
            val needs = crypto { batch.filter { store.needsOlm(it) } }
            val claim = buildJsonObject {
                needs.groupBy { it.userId() }.forEach { (user, keys) ->
                    put(user, buildJsonObject { keys.forEach { put(it.deviceId(), "signed_curve25519") } })
                }
            }
            val claimed = if (needs.isEmpty()) JsonObject(emptyMap()) else
                request("/api/e2ee/keys/claim", "POST", buildJsonObject { put("one_time_keys", claim) })["one_time_keys"]?.jsonObject
                    ?: JsonObject(emptyMap())
            val messages = buildJsonObject {
                batch.groupBy { it.userId() }.forEach { (user, keys) ->
                    put(user, buildJsonObject {
                        for (device in keys) {
                            val key = ((claimed[user] as? JsonObject)?.get(device.deviceId()) as? JsonObject)?.values?.singleOrNull()
                            val encrypted = crypto { store.shareKey(device, outbound, key?.toString()) }
                            checkNotNull(encrypted) { "受信端末の鍵を取得できませんでした" }
                            put(device.deviceId(), encrypted)
                        }
                    })
                }
            }
            val sent = request("/api/e2ee/to_device", "POST", buildJsonObject {
                put("device_id", store.deviceId)
                put("txn_id", UUID.randomUUID().toString())
                put("type", "m.room.encrypted")
                put("messages", messages)
            })
            check((sent["undelivered"] as? JsonArray).orEmpty().isEmpty()) { "受信端末に鍵を配れませんでした" }
        }
    }

    override suspend fun send(channel: String, id: String, parent: String?, body: String) =
        submit(channel, id, parent, body, 0uL, false)

    override suspend fun edit(message: Row, body: String) {
        check(message["ciphertext"] != null && message["ciphertext"] != JsonNull) { "e2ee.legacy.editForbidden" }
        submit(message.text("channel_id"), message.text("id"), message.text("parent_id").ifEmpty { null },
            body, message.text("revision").toULong() + 1uL, true, message.text("epoch").toULong(),
            message["ciphertext"].let { if (it is JsonPrimitive && it.isString) it.content else it.toString() },
            message.text("kind").ifEmpty { "message" })
    }

    private suspend fun submit(channel: String, id: String, parent: String?, body: String, revision: ULong, edit: Boolean, editEpoch: ULong? = null, editCiphertext: String? = null, editKind: String = "message") {
        sending.withLock {
            val store = store()
            var minimum = latestEpochs[channel] ?: 0uL
            repeat(3) {
                val (epoch, devices) = recipients(store, channel, minimum)
                latestEpochs[channel] = epoch
                try {
                    val now = System.currentTimeMillis().toULong()
                    val cached = outbound[channel]
                    if (cached != null && (cached.owner !== store || cached.session.needsRotation(now, epoch))) discard(channel)
                    val current = outbound.getOrPut(channel) { SendingSession(store, OutboundSession(channel, epoch, now)) }
                    val session = current.session
                    try {
                        val pending = devices.filter { deviceKey(it) !in current.shared }
                        share(store, session, pending)
                        current.shared += pending.map(::deviceKey)
                        val mentions = Mentions(emptyList(), false, false)
                        val previous = if (edit) crypto {
                            store.decrypt(EncryptedMessage(channel, id, parent, store.userId, editKind, editEpoch!!,
                                revision - 1uL, editCiphertext!!)).also { check(it.revision == RevisionStatus.Latest) { "e2ee.editConflict" } }
                        } else null
                        val kind = previous?.payload?.content?.kind ?: "message"
                        val attachments = previous?.payload?.attachments.orEmpty()
                        val encrypted = crypto { store.encrypt(session, OutgoingMessage(id, parent, revision,
                            MessageContent(body, "mfm", kind), mentions, attachments)) }
                        try {
                            request(if (edit) "/api/messages/$id" else "/api/messages", if (edit) "PATCH" else "POST", buildJsonObject {
                                if (!edit) { put("id", id); put("channel_id", channel) }
                                put("content", "")
                                put("kind", kind)
                                put("epoch", epoch.toLong())
                                put("revision", revision.toLong())
                                put("ciphertext", encrypted)
                                parent?.let { put("parent_id", it) }
                                if (!edit) put("mentions", buildJsonObject {
                                    put("user_ids", JsonArray(emptyList())); put("channel", false); put("here", false)
                                })
                            })
                            changes.value++
                            return@withLock
                        } catch (failure: ApiException) {
                            if (failure.code() != "E2EE_STALE_EPOCH") throw failure
                            discard(channel)
                            minimum = (failure.body?.get("current_epoch") as? JsonPrimitive)
                                ?.content?.toULongOrNull()?.let { maxOf(it, epoch) } ?: throw failure
                            latestEpochs[channel] = minimum
                        }
                    } catch (failure: Throwable) {
                        // 配布や保存に失敗したセッションを使って次の本文を送らない。
                        discard(channel)
                        throw failure
                    }
                } finally { devices.forEach { it.close() } }
            }
            discard(channel)
            error("e2ee.sendStaleEpoch")
        }
        // 既存のバックアップ処理が署名・版・失効の検査を行う。
        backups.trySend(Unit)
    }

    override suspend fun decrypt(message: Row): DecryptedBody = try {
        val store = ready.value
        if (store == null) DecryptedBody(notice = "e2ee.undecryptable.locked")
        else {
            val value = message["ciphertext"]
            val ciphertext = if (value is JsonPrimitive && value.isString) value.content else value.toString()
            val decrypted = crypto { store.decrypt(EncryptedMessage(
                message.text("channel_id"), message.text("id"), message.text("parent_id").ifEmpty { null },
                message.text("user_id"), message.text("kind").ifEmpty { "message" },
                message.text("epoch").toULong(), message.text("revision").toULong(), ciphertext)) }
            if (decrypted.revision != RevisionStatus.Latest) DecryptedBody(notice = "e2ee.undecryptable.invalid")
            else DecryptedBody(decrypted.payload.content.body, when {
                decrypted.origin == SessionOrigin.BACKUP -> "e2ee.sender.unauthenticated"
                !decrypted.senderVerified -> "e2ee.sender.unverified"
                else -> null
            })
        }
    } catch (failure: CancellationException) { throw failure }
    catch (_: ZooCryptoException.MissingRoomKey) { DecryptedBody(notice = "e2ee.undecryptable.keyMissing") }
    catch (_: Exception) { DecryptedBody(notice = "e2ee.undecryptable.invalid") }
}
