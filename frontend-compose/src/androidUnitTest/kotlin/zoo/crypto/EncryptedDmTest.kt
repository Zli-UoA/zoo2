package zoo.crypto

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpMethod
import io.ktor.http.HttpStatusCode
import io.ktor.http.content.ByteArrayContent
import io.ktor.http.content.OutgoingContent
import io.ktor.http.content.TextContent
import io.ktor.http.headersOf
import io.ktor.http.HttpHeaders
import java.io.File
import java.nio.file.Files
import javax.crypto.KeyGenerator
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import zoo.Config
import zoo.EncryptedMentions
import zoo.Platform
import zoo.RecipientNotice
import zoo.UndeliverableReason
import zoo.ZooApi
import zoo.fetchEncryptedAttachment
import zoo.uploadEncryptedAttachment

/**
 * 暗号化 DM の送受信（#361 の Android の分）。`frontend/e2e/e2ee.spec.ts` と同じ観点を、状態を持つ模擬サーバーと
 * 2 人（と暗号化を設定していない 1 人）の実際の暗号ストア（zoo-crypto の UniFFI）で確かめる。
 *
 * - 2 人が暗号化 DM でやり取りでき、サーバーに届く要求に平文が入らない（DB に平文が残らない）。
 * - 送る前に TOFU で信頼した相手のメッセージは「未検証の送信者」にならない。
 * - 鍵バックアップから取り込んだ端末では「送信者を確かめられません」になる。
 * - 退出したメンバーには、退出後のメッセージの鍵を配らない。
 * - 鍵を配れない相手（暗号化を設定していない）がいても送信は止めず、送信画面に理由を出す。増えた配布先も出す。
 * - 送信の要求の `mentions` は本文から作り、暗号化した添付は鍵を本文にだけ入れて、相手が取得して復号できる。
 *
 * Web（`@zoo/crypto-wasm`）と Android（UniFFI）は同じ `crates/zoo-crypto` の型と形式を使うので、ここで通る
 * 要求と暗号文の形は Web の相手とも同じ（docs/e2ee.md の「共有の暗号コア」）。
 */
class EncryptedDmTest {
    private val root: File = Files.createTempDirectory("zoo-e2ee-dm").toFile()
    private val opened = mutableListOf<CryptoStore>()
    private val lifetime = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val clients = mutableListOf<HttpClient>()

    @AfterTest
    fun cleanUp() {
        lifetime.cancel()
        clients.forEach { it.close() }
        opened.forEach { it.close() }
        root.deleteRecursively()
    }

    private val channel = "00000000-0000-4000-8000-0000000000c1"
    private val setups = mapOf(ALICE to FakeSetupServer(ALICE), BOB to FakeSetupServer(BOB), CAROL to FakeSetupServer(CAROL))

    /** 模擬サーバーの状態（チャンネルのメンバー、端末、to-device の受信箱、メッセージ、ピン、添付）。 */
    private val members = sortedSetOf(ALICE, BOB)
    private var epoch = 1
    private val devices = mutableMapOf<String, MutableList<Device>>()
    private val inbox = mutableMapOf<String, MutableList<ToDeviceRow>>()
    private val messages = mutableListOf<JsonObject>()
    private val pins = mutableMapOf<String, MutableMap<String, JsonElement>>()
    private val objects = mutableMapOf<String, ByteArray>()
    /** サーバーが受け取ったすべての要求の本文（平文が入らないことを確かめる）。 */
    private val wire = mutableListOf<String>()

    private inner class Device(val user: String, name: String) {
        private val wrapping = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
        private val files = CryptoStoreFiles(File(root, name), { wrapping }) {}
        val store: CryptoStore = CryptoStore.open(files, user)!!.store.also {
            if (!it.isRegistered) it.markRegistered()
            opened += it
            devices.getOrPut(user) { mutableListOf() } += this
        }
        val setup = E2eeSetup(setups.getValue(user), user, store = { store })
        val encryption = AndroidMessageEncryption(MutableStateFlow(store), ZooApi(Config("https://zoo.test", "/shapes", user, false), platform, client(user)), lifetime)

        /** 届いた to-device を処理する（Shape で受け取ったものと同じ）。 */
        fun receive() {
            val rows = inbox.remove("$user|${store.deviceId}").orEmpty()
            if (rows.isNotEmpty()) store.processToDevice(rows, snapshot(user).getValue("device_keys").jsonObject,
                snapshot(user).getValue("signatures").jsonArray, snapshot(user))
        }

        fun decrypt(row: JsonObject) = runBlocking { encryption.decrypt(row) }
    }

    private val platform = object : Platform {
        override suspend fun headers() = emptyMap<String, String>()
        override suspend fun readCache(key: String): String? = null
        override suspend fun writeCache(key: String, value: String) { error("No plaintext cache") }
        override fun uuid() = java.util.UUID.randomUUID().toString()
        override fun now() = "2026-01-01T00:00:00Z"
        override fun navigate(path: String) {}
        override fun translate(key: String, args: Map<String, String>) = key
        override fun action(name: String) {}
        override fun time(timestamp: String) = timestamp
        override fun day(timestamp: String) = timestamp
    }

    /** `keys/query` の形に `user_ids` と epoch を足した、呼び出したユーザーから見た配布先のスナップショット。 */
    private fun snapshot(caller: String): JsonObject = buildJsonObject {
        put("channel_id", channel)
        put("membership_epoch", epoch)
        put("user_ids", JsonArray(members.map(::JsonPrimitive)))
        put("device_keys", buildJsonObject {
            for (user in members) put(user, buildJsonObject {
                devices[user].orEmpty().forEach { put(it.store.deviceId, Json.parseToJsonElement(it.store.deviceKeys())) }
            })
        })
        for ((field, target) in listOf("master_key" to "master_keys", "self_signing_key" to "self_signing_keys")) {
            put(target, buildJsonObject { members.forEach { user -> setups.getValue(user).keys[field]?.let { put(user, it) } } })
        }
        put("user_signing_keys", buildJsonObject { setups.getValue(caller).keys["user_signing_key"]?.let { put(caller, it) } })
        put("signatures", JsonArray(members.flatMap { setups.getValue(it).signatures }))
        put("tofu_pins", JsonObject(pins[caller].orEmpty()))
    }

    private fun client(caller: String): HttpClient = HttpClient(MockEngine { request ->
        val bytes = when (val body = request.body) {
            is TextContent -> body.text.toByteArray()
            is ByteArrayContent -> body.bytes()
            is OutgoingContent.NoContent -> ByteArray(0)
            else -> ByteArray(0)
        }
        val text = bytes.decodeToString()
        synchronized(wire) { wire += text }
        val body = if (request.body is TextContent && text.isNotEmpty()) Json.parseToJsonElement(text).jsonObject else JsonObject(emptyMap())
        val path = request.url.encodedPath
        if (request.url.host == "s3.test") {
            return@MockEngine if (request.method == HttpMethod.Put) { objects[path] = bytes; respond("") }
            else respond(objects.getValue(path), HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/octet-stream"))
        }
        val response: JsonElement = synchronized(this@EncryptedDmTest) {
            when {
                path == "/api/e2ee/channels/$channel/recipients" -> {
                    if (caller !in members) return@MockEngine respond("""{"code":"NOT_FOUND"}""", HttpStatusCode.NotFound)
                    snapshot(caller)
                }
                path == "/api/e2ee/tofu_pins" -> {
                    body.getValue("pins").jsonArray.forEach { pin ->
                        val target = pin.jsonObject.getValue("target_user_id").jsonPrimitive.content
                        pins.getOrPut(caller) { mutableMapOf() }[target] = pin
                    }
                    JsonObject(emptyMap())
                }
                path == "/api/e2ee/keys/claim" -> buildJsonObject {
                    put("one_time_keys", buildJsonObject {
                        for ((user, wanted) in body.getValue("one_time_keys").jsonObject) put(user, buildJsonObject {
                            for (deviceId in wanted.jsonObject.keys) {
                                val device = devices[user].orEmpty().first { it.store.deviceId == deviceId }
                                val key = device.store.prepareKeyUpload(0, true)!!.oneTimeKeys.entries.first()
                                put(deviceId, JsonObject(mapOf(key.key to key.value)))
                            }
                        })
                    })
                }
                path == "/api/e2ee/to_device" -> {
                    for ((user, targets) in body.getValue("messages").jsonObject) {
                        for ((deviceId, content) in targets.jsonObject) {
                            val box = inbox.getOrPut("$user|$deviceId") { mutableListOf() }
                            box += ToDeviceRow("${caller}-${box.size}-${System.nanoTime()}", caller, "m.room.encrypted", content.jsonObject)
                        }
                    }
                    buildJsonObject { put("undelivered", JsonArray(emptyList())) }
                }
                path == "/api/messages" -> {
                    if (body.getValue("epoch").jsonPrimitive.content.toInt() != epoch) {
                        return@MockEngine respond("""{"code":"E2EE_STALE_EPOCH","current_epoch":$epoch}""", HttpStatusCode.Conflict)
                    }
                    messages += JsonObject(body + ("user_id" to JsonPrimitive(caller)))
                    JsonObject(emptyMap())
                }
                path == "/api/attachments/presign" -> {
                    val id = "00000000-0000-4000-8000-${"%012d".format(objects.size + 1)}"
                    buildJsonObject { put("data", buildJsonObject { put("attachment_id", id); put("upload_url", "https://s3.test/$id") }) }
                }
                path.startsWith("/api/attachments/") && path.endsWith("/url") -> {
                    val id = path.removePrefix("/api/attachments/").removeSuffix("/url")
                    buildJsonObject { put("data", buildJsonObject { put("download_url", "https://s3.test/$id") }) }
                }
                else -> error("Unexpected request: ${request.method.value} ${request.url}")
            }
        }
        respond(response.toString(), HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
    }).also { clients += it }

    /** 暗号化を設定して解錠した端末。`recoveryKey` があれば、その鍵で解錠する（2 台目以降）。 */
    private fun device(user: String, name: String, recoveryKey: String? = null): Pair<Device, String> = runBlocking {
        val device = Device(user, name)
        if (recoveryKey == null) {
            val start = device.setup.begin()
            device.setup.completeSetup(start.session, "t")
            start.session.close()
            device to start.recoveryKey
        } else {
            device.setup.unlock(UnlockMethod.Recovery(recoveryKey)).session.close()
            device to recoveryKey
        }
    }

    private fun sendAndDeliver(from: Device, body: String, mentions: EncryptedMentions = EncryptedMentions(),
        attachments: List<JsonObject> = emptyList()): JsonObject = runBlocking {
        from.encryption.send(channel, java.util.UUID.randomUUID().toString(), null, body, mentions, attachments)
        devices.values.flatten().forEach { it.receive() }
        messages.last()
    }

    @Test
    fun twoUsersExchangeEncryptedDmWithoutPlaintextOnTheWire() = runBlocking<Unit> {
        val (alice, aliceKey) = device(ALICE, "alice-1")
        val (bob, _) = device(BOB, "bob-1")

        val fromAlice = sendAndDeliver(alice, "SECRET-FROM-ALICE @bob", EncryptedMentions(listOf(BOB)))
        assertEquals("SECRET-FROM-ALICE @bob", bob.decrypt(fromAlice).body)
        // 送る前に TOFU で信頼した相手（Alice は送るときに Bob のピンを置いた）。
        val fromBob = sendAndDeliver(bob, "SECRET-FROM-BOB")
        val readByAlice = alice.decrypt(fromBob)
        assertEquals("SECRET-FROM-BOB", readByAlice.body)
        assertNull(readByAlice.notice, "TOFU で信頼した相手は「未検証の送信者」にならない")
        // 自分が送ったメッセージも読める。
        assertEquals("SECRET-FROM-ALICE @bob", alice.decrypt(fromAlice).body)

        // サーバーには平文を渡さない。`content` は空で、`mentions` だけを平文で送る。
        assertTrue(wire.none { "SECRET-FROM" in it })
        assertEquals("", fromAlice.getValue("content").jsonPrimitive.content)
        assertEquals(listOf(BOB), fromAlice.getValue("mentions").jsonObject.getValue("user_ids").jsonArray.map { it.jsonPrimitive.content })

        // 新しい端末で解錠すると、鍵バックアップから過去のセッションを取り込んで読める。送信者は確かめられない。
        alice.setup.uploadKeyBackup()
        val (aliceSecond, _) = device(ALICE, "alice-2", aliceKey)
        val restored = aliceSecond.decrypt(fromBob)
        assertEquals("SECRET-FROM-BOB", restored.body)
        assertEquals("e2ee.sender.unauthenticated", restored.notice)
        assertTrue(setups.getValue(ALICE).proofs.all { it != null }, "鍵バックアップの送信には証明を付ける")

        // 退出したメンバーには、退出後のメッセージの鍵を配らない。
        members.remove(BOB)
        epoch += 1
        inbox.clear()
        val afterLeave = sendAndDeliver(alice, "SECRET-AFTER-LEAVE")
        assertTrue(inbox.keys.none { it.startsWith("$BOB|") })
        assertEquals("e2ee.undecryptable.keyMissing", bob.decrypt(afterLeave).notice)
        assertEquals("SECRET-AFTER-LEAVE", aliceSecond.decrypt(afterLeave).body)
    }

    @Test
    fun undeliverableAndAddedRecipientsAreShownWithoutStoppingTheSend() = runBlocking<Unit> {
        val (alice, _) = device(ALICE, "alice-1")
        val (bob, _) = device(BOB, "bob-1")
        sendAndDeliver(alice, "first")
        // 初めて判定したチャンネルでは、増えた配布先を出さない。
        assertEquals(RecipientNotice(), alice.encryption.recipients.value[channel])

        // 暗号化を設定していない Carol が参加した。送信は止めず、Carol を「増えた配布先」と「配れない相手」に出す。
        members += CAROL
        epoch += 1
        devices[CAROL] = mutableListOf()
        val message = sendAndDeliver(alice, "SECRET-WITH-CAROL")
        assertEquals(RecipientNotice(listOf(CAROL), listOf(CAROL to UndeliverableReason.NOT_SET_UP)),
            alice.encryption.recipients.value[channel])
        assertEquals("SECRET-WITH-CAROL", bob.decrypt(message).body)

        // 次の判定では、増えた配布先は出さず、配れない相手だけを出す。
        sendAndDeliver(alice, "again")
        assertEquals(RecipientNotice(emptyList(), listOf(CAROL to UndeliverableReason.NOT_SET_UP)),
            alice.encryption.recipients.value[channel])
    }

    @Test
    fun encryptedAttachmentsRoundTripWithTheKeyOnlyInsideTheCiphertext() = runBlocking<Unit> {
        val (alice, _) = device(ALICE, "alice-1")
        val (bob, _) = device(BOB, "bob-1")
        val plaintext = "SECRET-FILE-CONTENT".encodeToByteArray()
        val api = ZooApi(Config("https://zoo.test", "/shapes", ALICE, false), platform, client(ALICE))
        val info = uploadEncryptedAttachment(api, alice.encryption.attachmentCipher, channel, plaintext, "secret.txt", "text/plain")
        val message = sendAndDeliver(alice, "SECRET-WITH-FILE", attachments = listOf(info))

        // 送信の要求には `attachment_ids` だけを入れ、鍵とファイル名は暗号化した本文に入れる。
        assertEquals(listOf(info.getValue("attachment_id")), message.getValue("attachment_ids").jsonArray.toList())
        assertTrue(wire.none { "secret.txt" in it || "SECRET-FILE-CONTENT" in it })
        assertTrue(objects.values.none { it.contentEquals(plaintext) })

        val decrypted = bob.decrypt(message)
        val received = decrypted.attachments.single()
        assertEquals("secret.txt", received.getValue("name").jsonPrimitive.content)
        val bobApi = ZooApi(Config("https://zoo.test", "/shapes", BOB, false), platform, client(BOB))
        assertContentEquals(plaintext, fetchEncryptedAttachment(bobApi, bob.encryption.attachmentCipher, received))

        // 暗号文が差し替えられたら、平文を返さない。
        objects.keys.forEach { objects[it] = objects.getValue(it).copyOf().also { bytes -> bytes[0] = (bytes[0] + 1).toByte() } }
        assertTrue(runCatching { fetchEncryptedAttachment(bobApi, bob.encryption.attachmentCipher, received) }.isFailure)
    }

    private companion object {
        const val ALICE = "00000000-0000-4000-8000-00000000a11c"
        const val BOB = "00000000-0000-4000-8000-000000000b0b"
        const val CAROL = "00000000-0000-4000-8000-0000000ca201"
    }
}
