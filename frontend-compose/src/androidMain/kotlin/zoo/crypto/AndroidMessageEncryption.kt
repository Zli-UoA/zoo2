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
    override val recipients = MutableStateFlow<Map<String, RecipientNotice>>(emptyMap())
    override val attachmentCipher: AttachmentCipher = UniffiAttachmentCipher
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
        // 初めて見た相手は、ピンを置けるまで配布先に入らない。置けたら同じスナップショットで判定し直す。
        for (round in 0..MAX_PIN_ROUNDS) {
            val distribution = crypto { store.recipients(pages) }
            var handedOff = false
            try {
                if (distribution.newPins.isEmpty() || round >= MAX_PIN_ROUNDS) {
                    // 配れない相手は配布先から外し、送信画面に理由を出す（Web の `EncryptionRecipientsNotice`）。
                    // 送信は止めない。
                    noteRecipients(store, channel, pages, distribution)
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

    private suspend fun noteRecipients(store: CryptoStore, channel: String, pages: List<JsonObject>, distribution: Distribution) {
        val members = pages.flatMap { page -> (page["user_ids"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonPrimitive)?.contentOrNull } }
        val added = crypto { store.noteRecipients(channel, members) }
        val undeliverable = distribution.users.entries.sortedBy { it.key }
            .mapNotNull { (user, trust) -> undeliverableReason(trust.state)?.let { user to it } }
        recipients.value = recipients.value + (channel to RecipientNotice(added, undeliverable))
    }

    /**
     * まだ配っていない端末に `zoo.room_key` を配り、配れた端末を返す。ワンタイム鍵もフォールバック鍵も払い出され
     * なかった端末と、to-device の `undelivered` の端末は返さないので、次の送信の前にまた配る（Web と同じ）。
     */
    private suspend fun share(store: CryptoStore, outbound: OutboundSession, devices: List<VerifiedDeviceKeys>): Set<String> {
        val delivered = mutableSetOf<String>()
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
            val sentTo = mutableListOf<VerifiedDeviceKeys>()
            val messages = buildJsonObject {
                batch.groupBy { it.userId() }.forEach { (user, keys) ->
                    val forUser = buildJsonObject {
                        for (device in keys) {
                            val key = ((claimed[user] as? JsonObject)?.get(device.deviceId()) as? JsonObject)?.values?.singleOrNull()
                            val encrypted = crypto { store.shareKey(device, outbound, key?.toString()) } ?: continue
                            put(device.deviceId(), encrypted)
                            sentTo += device
                        }
                    }
                    if (forUser.isNotEmpty()) put(user, forUser)
                }
            }
            if (messages.isEmpty()) continue
            val sent = request("/api/e2ee/to_device", "POST", buildJsonObject {
                put("device_id", store.deviceId)
                put("txn_id", UUID.randomUUID().toString())
                put("type", "m.room.encrypted")
                put("messages", messages)
            })
            val skipped = (sent["undelivered"] as? JsonArray).orEmpty().mapNotNull { target ->
                (target as? JsonObject)?.let { "${it.text("user_id")}|${it.text("device_id")}" }
            }.toSet()
            sentTo.filter { "${it.userId()}|${it.deviceId()}" !in skipped }.mapTo(delivered, ::deviceKey)
        }
        return delivered
    }

    override suspend fun send(
        channel: String, id: String, parent: String?, body: String,
        mentions: EncryptedMentions, attachments: List<JsonObject>,
    ) = submit(channel, id, parent, body, 0uL, false, mentions = mentions, newAttachments = attachments)

    override suspend fun edit(message: Row, body: String) {
        check(message["ciphertext"] != null && message["ciphertext"] != JsonNull) { "e2ee.legacy.editForbidden" }
        submit(message.text("channel_id"), message.text("id"), message.text("parent_id").ifEmpty { null },
            body, message.text("revision").toULong() + 1uL, true, message.text("epoch").toULong(),
            message["ciphertext"].let { if (it is JsonPrimitive && it.isString) it.content else it.toString() },
            message.text("kind").ifEmpty { "message" })
    }

    private suspend fun submit(
        channel: String, id: String, parent: String?, body: String, revision: ULong, edit: Boolean,
        editEpoch: ULong? = null, editCiphertext: String? = null, editKind: String = "message",
        mentions: EncryptedMentions = EncryptedMentions(), newAttachments: List<JsonObject> = emptyList(),
    ) {
        sending.withLock {
            val store = store()
            var minimum = latestEpochs[channel] ?: 0uL
            repeat(3) {
                val (epoch, devices) = recipients(store, channel, minimum)
                latestEpochs[channel] = epoch
                try {
                    val now = System.currentTimeMillis().toULong()
                    val cached = outbound[channel]
                    // 鍵を配った端末が配布先から外れたら（再検証が必要になった、配れなくなった、など）、その端末が
                    // 持つ鍵で次の本文を読めないよう、セッションを作り直す。
                    val distributed = devices.map(::deviceKey).toSet()
                    if (cached != null && (cached.owner !== store || cached.session.needsRotation(now, epoch) ||
                            !distributed.containsAll(cached.shared))) discard(channel)
                    val current = outbound.getOrPut(channel) { SendingSession(store, OutboundSession(channel, epoch, now)) }
                    val session = current.session
                    try {
                        val pending = devices.filter { deviceKey(it) !in current.shared }
                        current.shared += share(store, session, pending)
                        val previous = if (edit) crypto {
                            store.decrypt(EncryptedMessage(channel, id, parent, store.userId, editKind, editEpoch!!,
                                revision - 1uL, editCiphertext!!)).also { check(it.revision == RevisionStatus.Latest) { "e2ee.editConflict" } }
                        } else null
                        val kind = previous?.payload?.content?.kind ?: "message"
                        // 編集では、元の版の添付（鍵を含む）と mentions をそのまま入れる（Web の `editEncryptedMessage`）。
                        val attachments = previous?.payload?.attachments ?: newAttachments.map(::attachmentInfo)
                        val payloadMentions = previous?.payload?.mentions
                            ?: Mentions(mentions.userIds, mentions.channel, mentions.here)
                        val encrypted = crypto { store.encrypt(session, OutgoingMessage(id, parent, revision,
                            MessageContent(body, "mfm", kind), payloadMentions, attachments)) }
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
                                    put("user_ids", JsonArray(mentions.userIds.map(::JsonPrimitive)))
                                    put("channel", mentions.channel); put("here", mentions.here)
                                })
                                if (!edit && attachments.isNotEmpty()) {
                                    put("attachment_ids", JsonArray(attachments.map { JsonPrimitive(it.attachmentId) }))
                                }
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
            if (decrypted.revision is RevisionStatus.Stale) DecryptedBody(revision = MessageRevision.STALE)
            else if (decrypted.revision == RevisionStatus.Conflict) DecryptedBody(revision = MessageRevision.CONFLICT)
            else DecryptedBody(decrypted.payload.content.body, when {
                decrypted.origin == SessionOrigin.BACKUP -> "e2ee.sender.unauthenticated"
                !decrypted.senderVerified -> "e2ee.sender.unverified"
                else -> null
            }, attachments = decrypted.payload.attachments.map(::attachmentJson),
                mentions = decrypted.payload.mentions.let { EncryptedMentions(it.userIds.sorted(), it.channel, it.here) })
        }
    } catch (failure: CancellationException) { throw failure }
    catch (_: ZooCryptoException.MissingRoomKey) { DecryptedBody(notice = "e2ee.undecryptable.keyMissing") }
    catch (_: ZooCryptoException.Rejected) { DecryptedBody(notice = "e2ee.undecryptable.invalid") }
    catch (_: Exception) { DecryptedBody(notice = "e2ee.undecryptable.failed") }
}

/** 送信を止めずに配布先を判定し直す回数（ピンを置いたあと。Web の `MAX_PIN_ROUNDS`）。 */
private const val MAX_PIN_ROUNDS = 2

/** 配れない理由（Web の `undeliverableReason`）。配れる相手と、解錠し直しが要る自分は `null`。 */
internal fun undeliverableReason(state: UserState): UndeliverableReason? = when (state) {
    is UserState.Trusted, UserState.NeedsUnlock -> null
    UserState.NoCrossSigningKeys -> UndeliverableReason.NOT_SET_UP
    UserState.InvalidCrossSigningKeys -> UndeliverableReason.INVALID_KEYS
    is UserState.NeedsReverification -> UndeliverableReason.NEEDS_REVERIFICATION
    UserState.PinPending -> UndeliverableReason.PIN_PENDING
    UserState.Blocked -> UndeliverableReason.BLOCKED
}

/** 本文の `attachments` の JSON（[zoo.encryptedAttachmentInfo]）を UniFFI の値にする。 */
internal fun attachmentInfo(info: JsonObject): AttachmentInfo = AttachmentInfo(
    attachmentId = info.text("attachment_id"),
    name = info.text("name"),
    mimetype = info.text("mimetype"),
    size = info.getValue("size").jsonPrimitive.long.toULong(),
    width = (info["width"] as? JsonPrimitive)?.intOrNull?.toUInt(),
    height = (info["height"] as? JsonPrimitive)?.intOrNull?.toUInt(),
    sensitive = (info["sensitive"] as? JsonPrimitive)?.booleanOrNull == true,
    file = info.getValue("file").toString(),
)

/** UniFFI の添付を、表示と取得に使う JSON（[zoo.encryptedAttachmentInfo] と同じ形）にする。 */
internal fun attachmentJson(info: AttachmentInfo): JsonObject = encryptedAttachmentInfo(
    info.attachmentId, info.name, info.mimetype, info.size.toInt(),
    json.parseToJsonElement(info.file).jsonObject, info.width?.toInt(), info.height?.toInt(), info.sensitive,
)
