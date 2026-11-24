package zoo.crypto

import java.io.File
import java.io.IOException
import java.nio.file.Files
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import zoo.ApiException
import zoo.crypto.ffi.KeyUsage
import zoo.crypto.ffi.MessageContent
import zoo.crypto.ffi.Mentions
import zoo.crypto.ffi.OutboundSession
import zoo.crypto.ffi.OutgoingMessage
import zoo.crypto.ffi.RecoveryKey
import zoo.crypto.ffi.parseSecretStorage

/**
 * 失効（#362）の単体テストに使う、状態を持つ模擬サーバー（Web の `revocationController.test.ts` の `FakeServer`
 * と同じ規則）。自分のユーザー（[userId]）の SSSS、クロス署名鍵、署名、鍵バックアップの版と行、端末、ピンと、
 * 他のユーザーのマスター鍵（[peerMasters]）を持つ。前提の値（`expected_*`）と再認証を、実際のサーバーと同じく
 * 照合する。
 */
internal class FakeRevocationServer(val userId: String) : E2eeRevocationApi {
    class Request(val method: String, val path: String, val reauth: Boolean, val proof: String? = null)

    class Version(
        val version: String,
        val algorithm: JsonElement,
        val authData: JsonObject,
        var deleted: Boolean = false,
        var uploadKey: JsonElement? = JsonPrimitive(BackupProofTestKit.UPLOAD_KEY),
    ) {
        val publicKey: String get() = authData.getValue("public_key").jsonPrimitive.content
    }

    var defaultKeyId: String? = null
    var storageKey: JsonElement = JsonNull
    val secrets = mutableMapOf<String, JsonElement>()
    val slots = mutableListOf<JsonObject>()
    var revision = 0L
    var master: JsonElement? = null
    var selfSigning: JsonElement? = null
    var userSigning: JsonElement? = null
    val signatures = mutableListOf<JsonObject>()
    val versions = mutableListOf<Version>()
    val sessions = mutableMapOf<String, MutableList<JsonObject>>()
    val devices = mutableListOf<JsonObject>()
    val pins = linkedMapOf<String, JsonObject>()
    val peerMasters = mutableMapOf<String, JsonElement>()
    val requests = mutableListOf<Request>()
    val usedSlots = mutableListOf<String>()

    /** 置くと、その要求（`"POST /api/e2ee/cross_signing/signatures"` など）の直前に 1 回だけ通信を失敗させる。 */
    var fail: String? = null

    /** 置くと、その要求をサーバーに反映したあと、応答を返す前に 1 回だけ呼ぶ（割り込みを作るため）。 */
    var after: Pair<String, suspend () -> Unit>? = null

    private suspend fun <T> handle(method: String, path: String, reauth: String?, proof: String? = null, block: () -> T): T {
        requests += Request(method, path, reauth != null && reauth in acceptedTokens, proof)
        val route = "$method ${path.substringBefore('?')}"
        if (fail == route) {
            fail = null
            throw IOException("通信に失敗しました（$route）")
        }
        val result = block()
        after?.takeIf { it.first == route }?.let { (_, hook) ->
            after = null
            hook()
        }
        return result
    }

    /** 再認証として受け付ける ID トークン。 */
    val acceptedTokens = mutableSetOf(TOKEN_ACCEPTED)

    private fun error(status: Int, code: String, extra: JsonObject = JsonObject(emptyMap())): Nothing =
        throw ApiException(status, JsonObject(buildJsonObject { put("code", code); put("error", code) } + extra))

    private fun requireReauth(reauth: String?) {
        if (reauth == null || reauth !in acceptedTokens) error(403, "E2EE_REAUTH_REQUIRED")
    }

    private fun currentRevision(): Long? = if (defaultKeyId != null) revision else null

    private fun JsonObject.textOrNull(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull

    /** 前提にした既定の SSSS 鍵と `secrets_revision` を照合する（省略したものは照合しない）。 */
    private fun checkExpectedDefaultKey(body: JsonObject) {
        val keyMismatch = "expected_default_key_id" in body && body.textOrNull("expected_default_key_id") != defaultKeyId
        val revisionMismatch =
            "expected_secrets_revision" in body &&
                (body["expected_secrets_revision"] as? JsonPrimitive)?.longOrNull != currentRevision()
        if (keyMismatch || revisionMismatch) {
            error(409, "E2EE_SSSS_CONFLICT", buildJsonObject { put("default_key_id", defaultKeyId?.let(::JsonPrimitive) ?: JsonNull) })
        }
    }

    fun storage(): JsonObject = buildJsonObject {
        put("default_key_id", defaultKeyId?.let(::JsonPrimitive) ?: JsonNull)
        put("keys", if (defaultKeyId != null) buildJsonObject { put(defaultKeyId!!, storageKey) } else JsonObject(emptyMap()))
        put("secrets", JsonObject(secrets.toMap()))
        put("secrets_revision", currentRevision()?.let(::JsonPrimitive) ?: JsonNull)
        put("slots", JsonArray(slots.toList()))
    }

    override suspend fun secretStorage(): JsonObject = handle("GET", "/api/e2ee/secret_storage", null) { storage() }

    override suspend fun createSecretStorageKey(body: JsonObject, reauth: String?): Long? =
        handle("POST", "/api/e2ee/secret_storage/keys", reauth) {
            requireReauth(reauth)
            if (body.textOrNull("expected_default_key_id") != defaultKeyId ||
                (body["expected_secrets_revision"] as? JsonPrimitive)?.longOrNull != currentRevision()
            ) {
                error(409, "E2EE_SSSS_CONFLICT", buildJsonObject {
                    put("default_key_id", defaultKeyId?.let(::JsonPrimitive) ?: JsonNull)
                })
            }
            val keyId = body.getValue("key_id").jsonPrimitive.content
            defaultKeyId = keyId
            storageKey = body.getValue("key")
            secrets.clear()
            body.getValue("secrets").jsonObject.forEach { (name, secret) ->
                secrets[name] = JsonObject(secret.jsonObject + ("key_id" to JsonPrimitive(keyId)))
            }
            slots.clear()
            body.getValue("slots").jsonArray.forEach { slots += JsonObject(it.jsonObject + ("key_id" to JsonPrimitive(keyId))) }
            revision += 1
            revision
        }

    override suspend fun putSecrets(body: JsonObject, reauth: String?): Long? =
        handle("POST", "/api/e2ee/secret_storage/secrets", reauth) {
            val keyId = body.textOrNull("key_id")
            if (defaultKeyId == null || keyId != defaultKeyId) error(409, "E2EE_SSSS_CONFLICT")
            if ((body["expected_secrets_revision"] as? JsonPrimitive)?.longOrNull != revision) error(409, "E2EE_SSSS_CONFLICT")
            val incoming = body.getValue("secrets").jsonObject
            if (incoming.keys.any { it in secrets }) requireReauth(reauth)
            incoming.forEach { (name, secret) -> secrets[name] = JsonObject(secret.jsonObject + ("key_id" to JsonPrimitive(keyId!!))) }
            revision += 1
            revision
        }

    override suspend fun addKeySlot(slot: JsonObject, reauth: String?) {
        handle("POST", "/api/e2ee/secret_storage/slots", reauth) { if (slots.none { it["id"] == slot["id"] }) slots += slot }
    }

    override suspend fun markKeySlotUsed(slotId: String) {
        handle("POST", "/api/e2ee/secret_storage/slots/$slotId/used", null) { usedSlots += slotId }
    }

    override suspend fun listDevices(): JsonArray = handle("GET", "/api/e2ee/devices", null) { JsonArray(devices.toList()) }

    override suspend fun deleteDevice(deviceId: String) {
        handle("DELETE", "/api/e2ee/devices/$deviceId", null) {
            devices.removeAll { it["device_id"]?.jsonPrimitive?.content == deviceId }
            signatures.removeAll { it["target_key_id"]?.jsonPrimitive?.content == deviceId }
        }
    }

    override suspend fun tofuPins(after: String?): JsonObject =
        handle("GET", "/api/e2ee/tofu_pins", null) {
            buildJsonObject {
                put("pins", JsonArray(pins.values.toList()))
                put("next", JsonNull)
            }
        }

    /** `users` の `keys/query` の応答（自分のクロス署名鍵は、自分を含むときだけ）。 */
    fun keysQuery(users: List<String> = listOf(userId)): JsonObject = buildJsonObject {
        put("user_ids", JsonArray(users.distinct().map(::JsonPrimitive)))
        put("device_keys", JsonObject(emptyMap()))
        put("master_keys", buildJsonObject {
            for (user in users) (if (user == userId) master else peerMasters[user])?.let { put(user, it) }
        })
        put("self_signing_keys", buildJsonObject { if (userId in users) selfSigning?.let { put(userId, it) } })
        put("user_signing_keys", buildJsonObject { if (userId in users) userSigning?.let { put(userId, it) } })
        put("signatures", JsonArray(signatures.toList()))
        put("tofu_pins", buildJsonObject { users.forEach { user -> pins[user]?.let { put(user, it) } } })
    }

    override suspend fun queryOwnKeys(userId: String): JsonObject =
        handle("POST", "/api/e2ee/keys/query", null) { keysQuery(listOf(userId)) }

    override suspend fun queryKeys(userIds: List<String>): KeysQuery =
        handle("POST", "/api/e2ee/keys/query", null) {
            val raw = keysQuery(userIds)
            KeysQuery(raw.getValue("device_keys").jsonObject, raw.getValue("signatures").jsonArray, raw)
        }

    private fun Version.toJson(withUploadKey: Boolean, withDeleted: Boolean): JsonObject = buildJsonObject {
        put("version", version)
        put("algorithm", algorithm)
        put("auth_data", authData)
        if (withDeleted) put("deleted", deleted)
        if (withUploadKey) uploadKey?.let { put("upload_key", it) }
    }

    fun currentVersion(): Version? = versions.lastOrNull()?.takeIf { !it.deleted }

    fun liveVersions(): List<String> = versions.filter { !it.deleted }.map { it.version }

    // 一覧には `upload_key` を入れない（実際のサーバーと同じ）。
    override suspend fun backupVersions(): JsonObject =
        handle("GET", "/api/e2ee/key_backup/versions", null) {
            buildJsonObject { put("versions", JsonArray(versions.map { it.toJson(withUploadKey = false, withDeleted = true) })) }
        }

    override suspend fun backupVersion(version: String): JsonObject? =
        handle("GET", "/api/e2ee/key_backup/versions/$version", null) {
            versions.firstOrNull { it.version == version && !it.deleted }?.toJson(withUploadKey = true, withDeleted = false)
        }

    override suspend fun currentBackupVersion(): JsonObject? =
        handle("GET", "/api/e2ee/key_backup/versions/current", null) {
            currentVersion()?.toJson(withUploadKey = true, withDeleted = false)
        }

    override suspend fun deleteBackupVersion(version: String, reauth: String?) {
        handle("DELETE", "/api/e2ee/key_backup/versions/$version", reauth) {
            val target = versions.firstOrNull { it.version == version && !it.deleted } ?: return@handle
            if (target === currentVersion()) requireReauth(reauth)
            target.deleted = true
            sessions.remove(version)
        }
    }

    override suspend fun backupKeys(version: String, after: String?): JsonObject =
        handle("GET", "/api/e2ee/key_backup/keys?version=$version", null) {
            if (versions.none { it.version == version && !it.deleted }) error(404, "NOT_FOUND")
            buildJsonObject {
                put("sessions", JsonArray(sessions[version].orEmpty().toList()))
                put("next", JsonNull)
            }
        }

    override suspend fun uploadBackupKeys(version: String, secretsRevision: Long, body: JsonObject, proof: String?) {
        handle("POST", "/api/e2ee/key_backup/keys?version=$version&secrets_revision=$secretsRevision", null, proof) {
            val current = currentVersion()
            if (version != current?.version) error(409, "E2EE_WRONG_BACKUP_VERSION")
            if (secretsRevision != revision) error(409, "E2EE_SSSS_CONFLICT")
            if (proof != null && proof != BackupProofTestKit.expectedProof(current.publicKey, userId, version)) {
                error(403, "E2EE_BACKUP_PROOF_INVALID")
            }
            val rows = sessions.getOrPut(version) { mutableListOf() }
            for (session in body.getValue("sessions").jsonArray.map { it.jsonObject }) {
                rows.removeAll { row -> listOf("channel_id", "sender_key", "session_id").all { row[it] == session[it] } }
                rows += session
            }
        }
    }

    private fun keyOf(key: JsonElement?): String? =
        (key as? JsonObject)?.get("keys")?.jsonObject?.values?.singleOrNull()?.jsonPrimitive?.content

    override suspend fun uploadCrossSigningKeys(body: JsonObject, reauth: String?) {
        handle("POST", "/api/e2ee/cross_signing/keys", reauth) {
            requireReauth(reauth)
            checkExpectedDefaultKey(body)
            val expected = body["expected_keys"] as? JsonObject ?: JsonObject(emptyMap())
            if (expected.textOrNull("master") != keyOf(master) ||
                expected.textOrNull("self_signing") != keyOf(selfSigning) ||
                expected.textOrNull("user_signing") != keyOf(userSigning)
            ) {
                error(409, "E2EE_CROSS_SIGNING_CONFLICT")
            }
            val replaced = listOfNotNull(keyOf(master), keyOf(selfSigning), keyOf(userSigning))
            body["master_key"]?.let { master = it }
            body["self_signing_key"]?.let { selfSigning = it }
            body["user_signing_key"]?.let { userSigning = it }
            // 鍵を変えたら、古い鍵による署名を消す。
            val live = listOf(keyOf(master), keyOf(selfSigning), keyOf(userSigning))
            val removed = replaced.filter { it !in live }
            signatures.removeAll { row -> removed.any { row["signer_key_id"]?.jsonPrimitive?.content == "ed25519:$it" } }
        }
    }

    private fun addSignatures(uploads: JsonArray) {
        for (upload in uploads.map { it.jsonObject }) {
            val target = upload.getValue("target_user_id").jsonPrimitive.content
            // 他のユーザーのマスター鍵への署名は、今のマスター鍵へのものだけを受け付ける（作り直されていれば 404）。
            if (target != userId) {
                val current = keyOf(peerMasters[target])
                if (current == null || upload.getValue("target_key_id").jsonPrimitive.content != current) error(404, "NOT_FOUND")
            }
            signatures += JsonObject(buildJsonObject { put("signer_user_id", userId) } + upload)
        }
    }

    override suspend fun uploadSignatures(signatures: JsonArray) {
        handle("POST", "/api/e2ee/cross_signing/signatures", null) { addSignatures(signatures) }
    }

    override suspend fun uploadSignaturesWithReauth(signatures: JsonArray, reauth: String?) {
        handle("POST", "/api/e2ee/cross_signing/signatures", reauth) { addSignatures(signatures) }
    }

    override suspend fun uploadPins(pins: JsonArray, replace: Boolean, reauth: String?) {
        handle("POST", "/api/e2ee/tofu_pins", reauth) {
            if (replace) requireReauth(reauth)
            for (pin in pins.map { it.jsonObject }) {
                val target = pin.getValue("target_user_id").jsonPrimitive.content
                if (!replace && target in this.pins) error(409, "E2EE_PIN_CONFLICT")
                this.pins[target] = JsonObject(buildJsonObject { put("type", "zoo.tofu_pin"); put("user_id", userId) } + pin)
            }
        }
    }

    override suspend fun createBackupVersion(body: JsonObject, reauth: String?) {
        createBackupVersionNumber(body, reauth)
    }

    override suspend fun createBackupVersionNumber(body: JsonObject, reauth: String?): String? =
        handle("POST", "/api/e2ee/key_backup/versions", reauth) {
            checkExpectedDefaultKey(body)
            val current = currentVersion()
            val authData = body.getValue("auth_data").jsonObject
            if (current != null && current.publicKey == authData.getValue("public_key").jsonPrimitive.content) {
                return@handle current.version
            }
            if (body.textOrNull("expected_current_version") != current?.version) error(409, "E2EE_WRONG_BACKUP_VERSION")
            if (versions.isNotEmpty()) requireReauth(reauth)
            val version = (versions.size + 1).toString()
            versions += Version(version, body.getValue("algorithm"), authData)
            version
        }

    /** 置いた SSSS をリカバリーキーで開けて、退避した古いバックアップ鍵の公開鍵を返す。 */
    fun previousBackupKeys(recoveryKey: String): List<String> =
        parseSecretStorage(userId, storage().toString())!!.use { storage ->
            RecoveryKey.parse(recoveryKey).use { recovery ->
                storage.unlockWithRecoveryKey(recovery).use { unlocked ->
                    unlocked.previousBackupKeys().map { key -> key.use { it.publicKey() } }
                }
            }
        }

    fun register(device: RevokingDevice, at: String = "2026-11-01T00:00:00Z") {
        devices += buildJsonObject {
            put("device_id", device.store.deviceId)
            put("display_name", device.name)
            put("device_keys", JsonObject(emptyMap()))
            put("created_at", at)
            put("last_seen_at", at)
        }
    }

    companion object {
        const val TOKEN_ACCEPTED = "reauth-id-token"
    }
}

/** 失効のテストの 1 つの端末（暗号ストアと、それを使う初回設定と失効）。`restart` でメモリーの秘密を失う。 */
internal class RevokingDevice(val name: String, root: File, val server: FakeRevocationServer) {
    private val wrapping: SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    private val files = CryptoStoreFiles(File(root, name), { wrapping }) {}
    var store: CryptoStore = openStore()
        private set
    val setup = E2eeSetup(server, server.userId, store = { store })
    var revocation = E2eeRevocation(server, server.userId, store = { store })
        private set

    private fun openStore(): CryptoStore =
        CryptoStore.open(files, server.userId)!!.store.also { if (!it.isRegistered) it.markRegistered() }

    /** アプリを終了して開き直す（失効と SSSS のメモリーの秘密は消え、暗号ストアの進み具合は残る）。 */
    fun restart() {
        revocation.dispose()
        store.close()
        store = openStore()
        revocation = E2eeRevocation(server, server.userId, store = { store })
    }

    fun close() {
        revocation.dispose()
        store.close()
    }
}

/** 失効のテストの環境。初回設定を済ませ、今の版に [sessionCount] 件のセッションを送った端末を作る。 */
internal class RevocationWorld(val userId: String = ALICE) : AutoCloseable {
    val root: File = Files.createTempDirectory("zoo-e2ee-revocation").toFile()
    val server = FakeRevocationServer(userId)
    private val devices = mutableListOf<RevokingDevice>()

    fun device(name: String, at: String = "2026-11-01T00:00:00Z"): RevokingDevice =
        RevokingDevice(name, root, server).also {
            devices += it
            server.register(it, at)
        }

    /** 初回設定を済ませた端末と、そのリカバリーキー。 */
    suspend fun setUp(sessionCount: Int = 3): Pair<RevokingDevice, String> {
        val first = device("first")
        val start = first.setup.begin()
        first.setup.completeSetup(start.session, FakeRevocationServer.TOKEN_ACCEPTED)
        start.session.close()
        // この端末で送ったセッションを、今の版（1）に証明付きで送る。
        repeat(sessionCount) { index ->
            OutboundSession("00000000-0000-4000-8000-0000000000c${index + 1}", 1uL, 0uL).use { outbound ->
                first.store.encrypt(
                    outbound,
                    OutgoingMessage(
                        "00000000-0000-4000-8000-0000000000d${index + 1}", null, 0uL,
                        MessageContent("body", "mfm", "message"), Mentions(emptyList(), false, false),
                    ),
                )
            }
        }
        if (sessionCount > 0) first.setup.uploadKeyBackup()
        return first to start.recoveryKey
    }

    /** SSSS をリカバリーキーで開けて、自分のユーザー署名鍵を取り出す（テストで記録を作るため）。 */
    fun userSigning(recoveryKey: String) =
        parseSecretStorage(userId, server.storage().toString())!!.use { storage ->
            RecoveryKey.parse(recoveryKey).use { recovery ->
                storage.unlockWithRecoveryKey(recovery).use { it.crossSigningKey(KeyUsage.USER_SIGNING)!! }
            }
        }

    override fun close() {
        devices.forEach { it.close() }
        root.deleteRecursively()
    }

    companion object {
        const val ALICE = "00000000-0000-4000-8000-00000000000a"
        const val BOB = "00000000-0000-4000-8000-00000000000b"
        const val CAROL = "00000000-0000-4000-8000-00000000000c"

        fun parse(value: String): JsonObject = Json.parseToJsonElement(value).jsonObject
    }
}
