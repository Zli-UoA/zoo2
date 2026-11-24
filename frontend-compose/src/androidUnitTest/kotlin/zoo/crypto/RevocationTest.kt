package zoo.crypto

import java.io.IOException
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertIs
import kotlin.test.assertNotEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import zoo.crypto.RevocationWorld.Companion.ALICE
import zoo.crypto.RevocationWorld.Companion.BOB
import zoo.crypto.RevocationWorld.Companion.CAROL
import zoo.crypto.RevocationWorld.Companion.parse
import zoo.crypto.ffi.CrossSigningKeyPair
import zoo.crypto.ffi.KeyUsage
import zoo.crypto.ffi.ResignNote
import zoo.crypto.ffi.RevocationScope
import zoo.crypto.ffi.RevocationStage
import zoo.crypto.ffi.ZooCryptoException

/**
 * 失効の手順（#362）。React の `revocationController.test.ts` と同じく、本物の zoo-crypto（UniFFI）と状態を持つ
 * 模擬サーバー（[FakeRevocationServer]）で、要求の順、段ごとの保存と再開、再認証、409 のやり直しを確かめる。
 */
class RevocationTest {
    private val world = RevocationWorld()
    private val server get() = world.server

    @AfterTest
    fun cleanUp() = world.close()

    private val token = FakeRevocationServer.TOKEN_ACCEPTED

    private fun keyOf(key: kotlinx.serialization.json.JsonElement?): String? =
        (key as? JsonObject)?.get("keys")?.jsonObject?.values?.single()?.jsonPrimitive?.content

    private fun migrationPosts(version: String) =
        server.requests.filter { it.method == "POST" && it.path.startsWith("/api/e2ee/key_backup/keys?version=$version&") }

    /** 署名付きの JSON に、自分の `keyId` の署名があるか。 */
    private fun signedBy(value: JsonObject, keyId: String): Boolean =
        value["signatures"]?.jsonObject?.get(ALICE)?.jsonObject?.containsKey(keyId) == true

    @Test
    fun fullRevocationReplacesEveryKeyMigratesTheBackupAndResignsConfirmedPeers() = runBlocking<Unit> {
        val (first, recoveryKey) = world.setUp()
        val other = world.device("other")
        other.setup.unlock(UnlockMethod.Recovery(recoveryKey)).session.close()
        val lost = world.device("lost", "2026-11-02T00:00:00Z")
        val lostId = lost.store.deviceId

        // Bob は受け入れて（再検証）、confirmed_usk のある記録とピン、マスター鍵への署名がある。
        val usk = world.userSigning(recoveryKey)
        val bob = CrossSigningKeyPair.generate(BOB, KeyUsage.MASTER)
        server.peerMasters[BOB] = parse(bob.publicKeyJson())
        val accepted = first.store.trustStore.acceptMasterKey(usk, BOB, bob.publicKeyJson())
        server.pins[BOB] = parse(accepted.pin)
        server.signatures += buildJsonObject {
            put("signer_user_id", ALICE)
            put("signer_key_id", accepted.signature.signerKeyId)
            put("target_user_id", accepted.signature.targetUserId)
            put("target_key_id", accepted.signature.targetKeyId)
            put("signature", accepted.signature.signature)
        }
        first.store.trustStore.confirmAcceptedMasterKey(BOB, bob.publicKey())
        // Carol は、今のマスター鍵と違う共有ピンを採用しただけで、confirmed_usk のない記録。
        val carol = CrossSigningKeyPair.generate(CAROL, KeyUsage.MASTER)
        val carolPinned = CrossSigningKeyPair.generate(CAROL, KeyUsage.MASTER)
        server.peerMasters[CAROL] = parse(carol.publicKeyJson())
        val carolPin = parse(usk.signTofuPin(CAROL, carolPinned.publicKey()))
        server.pins[CAROL] = carolPin
        first.store.trustStore.evaluate(listOf(server.keysQuery(listOf(CAROL, ALICE)).toString()), first.store.deviceId, usk)
            .devices.forEach { it.close() }
        first.store.saveTrustStore()
        assertNull(first.store.trustStore.record(CAROL)!!.confirmedUsk)
        assertTrue(first.store.trustStore.record(BOB)!!.confirmedUsk != null)

        val oldMaster = keyOf(server.master)
        val oldKeyId = server.defaultKeyId
        val oldBackup = server.versions[0].publicKey

        val revocation = first.revocation
        revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        revocation.deleteSelectedDevices(listOf(lostId))
        val begun = revocation.begin(RevocationScope.FULL, listOf(lostId))
        assertEquals(RevocationScope.FULL, begun.scope)
        assertEquals(RevocationStage.PREPARED, begun.stage)
        assertEquals(listOf(lostId), begun.deletedDevices)
        assertEquals(listOf(BOB), begun.resignedUsers)
        assertEquals(listOf(BOB), begun.resignedMasterKeys)
        assertTrue(ResignNote.NOT_CONFIRMED in begun.notes.getValue(CAROL))
        assertEquals(listOf(CAROL), begun.notResignedUsers().map { it.userId })
        // 始めただけでは、サーバーに何も保存しない。終わるまで、ログアウトの前に警告する。
        assertEquals(oldMaster, keyOf(server.master))
        assertTrue(first.store.hasPendingRevocation)
        assertTrue(revocation.isPending())
        assertEquals(begun, revocation.summary.value)

        val newRecoveryKey = revocation.newRecoveryKey()
        val stages = mutableListOf<RevocationStage>()
        var reauths = 0
        // 再認証を済ませずに始めると、最初の保存の 403 で 1 回だけ再認証してから続ける。
        val result = revocation.continueRevocation(null, reauthenticate = { reauths++; token }) { stages += it.stage }
        assertEquals(1, reauths)
        assertEquals(
            listOf(
                RevocationStage.SECRET_STORAGE_SAVED, RevocationStage.CROSS_SIGNING_SAVED, RevocationStage.SIGNATURES_SAVED,
                RevocationStage.BACKUP_CREATED, RevocationStage.MIGRATED,
            ),
            stages.distinct(),
        )
        assertEquals(RevocationStage.VERIFIED, result.summary.stage)
        assertEquals(0L, result.droppedSessions)

        // マスター鍵、SSSS 鍵を作り直し、版 2 に切り替えて、古い版のセッションを移して消した。
        assertNotEquals(oldMaster, keyOf(server.master))
        assertNotEquals(oldKeyId, server.defaultKeyId)
        assertEquals(listOf("2"), server.liveVersions())
        assertEquals(3, server.sessions["2"]?.size)
        val posts = migrationPosts("2")
        assertTrue(posts.isNotEmpty())
        val proof = BackupProofTestKit.expectedProof(server.versions.last().publicKey, ALICE, "2")
        assertEquals(posts.map { proof }, posts.map { it.proof })
        assertTrue(oldBackup in server.previousBackupKeys(newRecoveryKey))
        // 鍵を差し替える要求は、403 のあとはすべて再認証付き。
        val keyed = server.requests.filter {
            it.method == "POST" && it.path in listOf("/api/e2ee/cross_signing/keys", "/api/e2ee/key_backup/versions")
        }
        assertTrue(keyed.isNotEmpty() && keyed.all { it.reauth })

        // 削除した端末は消え、解錠し直していない端末は新しい自己署名鍵の署名がない。
        assertTrue(server.devices.none { it["device_id"]?.jsonPrimitive?.content == lostId })
        assertEquals(listOf(other.store.deviceId), result.unsignedDevices.map { it.deviceId })
        assertNull(revocation.status())
        assertFalse(first.store.hasPendingRevocation)
        assertNull(revocation.summary.value)

        // Bob のピンとマスター鍵への署名は、新しいユーザー署名鍵で付け直した。Carol のピンはそのまま。
        val newUsk = "ed25519:${keyOf(server.userSigning)}"
        assertTrue(signedBy(server.pins.getValue(BOB), newUsk))
        assertTrue(server.signatures.any {
            it["signer_key_id"]?.jsonPrimitive?.content == newUsk && it["target_user_id"]?.jsonPrimitive?.content == BOB
        })
        assertEquals(carolPin, server.pins.getValue(CAROL))
        assertFalse(signedBy(server.pins.getValue(CAROL), newUsk))

        // 失効した端末は、新しい鍵と secrets_revision を記録した（解錠し直さずに使える）。
        assertEquals(server.revision, first.store.secretsRevision)
        assertIs<E2eeStatus.Unlocked>(first.setup.loadStatus())
        assertEquals(keyOf(server.master), first.store.trustStore.ownMasterKey())

        // ほかの端末は解錠し直しを求められ、古いリカバリーキーでは開けず、新しいリカバリーキーで開ける。
        assertEquals(LockReason.MISMATCH, (other.setup.loadStatus() as E2eeStatus.Locked).reason)
        assertFailsWith<ZooCryptoException.KeySlotUnlock> { other.setup.unlock(UnlockMethod.Recovery(recoveryKey)) }
        other.setup.unlock(UnlockMethod.Recovery(newRecoveryKey)).session.close()
        assertIs<E2eeStatus.Unlocked>(other.setup.loadStatus())
        assertTrue(first.revocation.loadDevices(first.store.deviceId).all { it.signed })

        // 新しい端末は、新しい版に移したセッションを取り込める。
        val third = world.device("third")
        third.setup.unlock(UnlockMethod.Recovery(newRecoveryKey)).session.close()
        assertEquals(3, third.store.inboundSessionCount)
    }

    @Test
    fun deviceOnlyRevocationKeepsTheMasterKeyAndMovesTheOldVersion() = runBlocking<Unit> {
        val (first, recoveryKey) = world.setUp()
        val lost = world.device("lost", "2026-11-02T00:00:00Z")
        val oldMaster = keyOf(server.master)
        val oldSelfSigning = keyOf(server.selfSigning)
        val oldKeyId = server.defaultKeyId
        val oldBackup = server.versions[0].publicKey
        val revocation = first.revocation

        val devices = revocation.loadDevices(first.store.deviceId)
        assertEquals(
            listOf(Triple(first.store.deviceId, true, true), Triple(lost.store.deviceId, false, false)),
            devices.map { Triple(it.deviceId, it.current, it.signed) },
        )

        revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        revocation.deleteSelectedDevices(listOf(lost.store.deviceId))
        val begun = revocation.begin(RevocationScope.DEVICE_ONLY, listOf(lost.store.deviceId))
        assertEquals(RevocationStage.PREPARED, begun.stage)
        assertEquals(oldKeyId, begun.keyId)
        assertEquals(oldSelfSigning, keyOf(server.selfSigning))

        val stages = mutableListOf<RevocationStage>()
        val result = revocation.continueRevocation(token) { stages += it.stage }
        assertEquals(
            listOf(
                RevocationStage.SECRET_STORAGE_SAVED, RevocationStage.CROSS_SIGNING_SAVED, RevocationStage.SIGNATURES_SAVED,
                RevocationStage.BACKUP_CREATED, RevocationStage.MIGRATED,
            ),
            stages,
        )
        assertEquals(0L, result.droppedSessions)
        assertEquals(emptyList(), result.unsignedDevices)
        assertEquals(oldMaster, keyOf(server.master))
        assertEquals(oldKeyId, server.defaultKeyId)
        assertNotEquals(oldSelfSigning, keyOf(server.selfSigning))
        assertEquals(begun.revocationId, keyOf(server.selfSigning))
        assertEquals(listOf("2"), server.liveVersions())
        assertEquals(3, server.sessions["2"]?.size)
        val proof = BackupProofTestKit.expectedProof(server.versions.last().publicKey, ALICE, "2")
        assertTrue(migrationPosts("2").let { posts -> posts.isNotEmpty() && posts.all { it.proof == proof } })
        // 古い版の鍵は SSSS に退避した（同じリカバリーキーで開ける）。
        assertTrue(oldBackup in server.previousBackupKeys(recoveryKey))
        assertNull(revocation.status())
        // 鍵を差し替える要求はすべて再認証付き。
        assertTrue(server.requests.filter {
            it.method == "POST" && it.path in listOf(
                "/api/e2ee/secret_storage/secrets", "/api/e2ee/cross_signing/keys", "/api/e2ee/key_backup/versions",
            )
        }.all { it.reauth })
        assertIs<E2eeStatus.Unlocked>(first.setup.loadStatus())
    }

    /** `stop` の要求の直前で通信が切れ、アプリを開き直して SSSS を開け直し、記録した段から最後まで続ける。 */
    private suspend fun stopAndResume(scope: RevocationScope, stop: String) {
        RevocationWorld().use { world ->
            val server = world.server
            val (first, recoveryKey) = world.setUp()
            first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
            val begun = first.revocation.begin(scope, emptyList())
            val unlockWith = if (scope == RevocationScope.FULL) first.revocation.newRecoveryKey() else recoveryKey
            server.fail = stop
            assertFailsWith<IOException>(stop) { first.revocation.continueRevocation(token) }

            // アプリを開き直す（メモリーの秘密を失い、暗号ストアの進み具合は残る）。
            first.restart()
            // 画面と同じく、開き直したときの状態の読み込みと起動時の確認を挟む。SSSS を保存したあとの段で止まって
            // secrets_revision が変わっていても、記録した値を消さず、失効の記録も残る（「失効を続ける」を出せる）。
            val recorded = first.store.secretsRevision
            first.setup.loadStatus()
            first.setup.runStartupCheck()
            assertEquals(recorded, first.store.secretsRevision, stop)
            val saved = first.revocation.status()!!
            assertEquals(begun.revocationId, saved.revocationId, stop)
            assertTrue(first.store.hasPendingRevocation, stop)
            // 続ける前に、SSSS を開け直す（開けずに続けると止める）。
            assertFailsWith<RevocationException.NothingToContinue>(stop) { first.revocation.continueRevocation(token) }
            first.revocation.openSecretStorage(UnlockMethod.Recovery(unlockWith))
            val resumed = assertIs<RevocationResume.Resumed>(first.revocation.resume(saved.revocationId), stop)
            assertEquals(saved.stage, resumed.summary.stage, stop)
            first.revocation.continueRevocation(token)
            assertEquals(listOf("2"), server.liveVersions(), stop)
            assertEquals(3, server.sessions["2"]?.size, stop)
            assertNull(first.revocation.status(), stop)
            assertEquals(begun.revocationId, keyOf(server.selfSigning), stop)
            assertIs<E2eeStatus.Unlocked>(first.setup.loadStatus(), stop)
        }
    }

    @Test
    fun deviceOnlyRevocationResumesFromEveryStage() = runBlocking<Unit> {
        for (stop in listOf(
            "POST /api/e2ee/secret_storage/secrets",
            "POST /api/e2ee/cross_signing/keys",
            "POST /api/e2ee/cross_signing/signatures",
            "POST /api/e2ee/key_backup/versions",
            "GET /api/e2ee/key_backup/keys",
            "DELETE /api/e2ee/key_backup/versions/1",
            "GET /api/e2ee/devices",
        )) stopAndResume(RevocationScope.DEVICE_ONLY, stop)
    }

    @Test
    fun fullRevocationResumesFromEveryStageAfterSavingSecretStorage() = runBlocking<Unit> {
        for (stop in listOf(
            "POST /api/e2ee/cross_signing/keys",
            "POST /api/e2ee/cross_signing/signatures",
            "POST /api/e2ee/key_backup/versions",
            "GET /api/e2ee/key_backup/keys",
            "DELETE /api/e2ee/key_backup/versions/1",
            "GET /api/e2ee/devices",
        )) stopAndResume(RevocationScope.FULL, stop)
    }

    @Test
    fun reauthRequiredStopsAndRetriesWithTheSameKeys() = runBlocking<Unit> {
        val (first, recoveryKey) = world.setUp()
        val revocation = first.revocation
        revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        val begun = revocation.begin(RevocationScope.DEVICE_ONLY, emptyList())
        val revision = server.revision
        assertFailsWith<RevocationException.ReauthRequired> { revocation.continueRevocation(null) }
        // 再認証を取りやめた（`null`）ときも止まる。何も保存していない。
        assertFailsWith<RevocationException.ReauthRequired> { revocation.continueRevocation(null, reauthenticate = { null }) }
        assertEquals(revision, server.revision)
        assertEquals(RevocationStage.PREPARED, revocation.status()!!.stage)
        // 期限の切れたトークンも、1 回だけ再認証して続ける。
        val tokens = mutableListOf<String>()
        revocation.continueRevocation("expired", reauthenticate = { token.also(tokens::add) })
        assertEquals(listOf(token), tokens)
        assertEquals(begun.revocationId, keyOf(server.selfSigning))
        assertEquals(listOf("2"), server.liveVersions())
    }

    @Test
    fun anotherDevicesRevocationIsAConflictAndRestartsAfterAbandoning() = runBlocking<Unit> {
        val (first, recoveryKey) = world.setUp()
        val other = world.device("other")
        other.setup.unlock(UnlockMethod.Recovery(recoveryKey)).session.close()
        other.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        val begun = first.revocation.begin(RevocationScope.DEVICE_ONLY, emptyList())
        other.revocation.begin(RevocationScope.DEVICE_ONLY, emptyList())
        other.revocation.continueRevocation(token)
        val conflict = assertFailsWith<RevocationException.Conflict> { first.revocation.continueRevocation(token) }
        assertEquals("E2EE_SSSS_CONFLICT", conflict.code)

        // 記録を捨て、SSSS を開け直してやり直す。
        first.revocation.abandon(begun.revocationId)
        assertNull(first.revocation.status())
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        first.revocation.begin(RevocationScope.DEVICE_ONLY, emptyList())
        first.revocation.continueRevocation(token)
        assertEquals(listOf("3"), server.liveVersions())
        assertEquals(3, server.sessions["3"]?.size)
        assertIs<E2eeStatus.Unlocked>(first.setup.loadStatus())
    }

    @Test
    fun secretStorageRecreatedByAnotherDeviceStopsBeforeWritingKeys() = runBlocking<Unit> {
        for (stop in listOf("POST /api/e2ee/cross_signing/keys", "POST /api/e2ee/key_backup/versions")) {
            for (scope in listOf(RevocationScope.FULL, RevocationScope.DEVICE_ONLY)) {
                RevocationWorld().use { world ->
                    val server = world.server
                    val (first, recoveryKey) = world.setUp()
                    val other = world.device("other")
                    first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
                    first.revocation.begin(scope, emptyList())
                    val recoveryOfA = if (scope == RevocationScope.FULL) first.revocation.newRecoveryKey() else recoveryKey
                    server.fail = stop
                    assertFailsWith<IOException> { first.revocation.continueRevocation(token) }

                    // B の失効が SSSS だけを保存して止まる。
                    other.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryOfA))
                    other.revocation.begin(scope, emptyList())
                    if (scope == RevocationScope.FULL) other.revocation.newRecoveryKey()
                    server.fail = "POST /api/e2ee/cross_signing/keys"
                    assertFailsWith<IOException> { other.revocation.continueRevocation(token) }
                    val keyOfB = server.defaultKeyId
                    val revisionOfB = server.revision
                    val selfSigning = keyOf(server.selfSigning)
                    val master = keyOf(server.master)
                    val versions = server.versions.size

                    val conflict = assertFailsWith<RevocationException.Conflict>("$stop $scope") {
                        first.revocation.continueRevocation(token)
                    }
                    assertEquals("E2EE_SSSS_CONFLICT", conflict.code)
                    assertEquals(master, keyOf(server.master))
                    assertEquals(versions, server.versions.size)
                    assertEquals(keyOfB, server.defaultKeyId)
                    assertEquals(revisionOfB, server.revision)
                    assertEquals(selfSigning, keyOf(server.selfSigning))
                }
            }
        }
    }

    @Test
    fun fullRevocationStoppedBeforeSavingSecretStorageRestartsWithTheSameScope() = runBlocking<Unit> {
        val (first, recoveryKey) = world.setUp()
        val oldMaster = keyOf(server.master)
        val oldKeyId = server.defaultKeyId
        val lost = "00000000-0000-4000-8000-0000000000ff"
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        first.revocation.begin(RevocationScope.FULL, listOf(lost))
        // SSSS を保存する前に止めた。マスター鍵と SSSS 鍵は保存しないので続けられない。
        first.restart()
        assertEquals(RevocationStage.PREPARED, first.revocation.status()!!.stage)
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        // やり直しでも、記録した範囲（1〜5）と削除した端末を引き継ぐ（端末だけに縮めない）。
        assertEquals(RevocationResume.Restart(RevocationScope.FULL, listOf(lost)), first.revocation.resume())
        assertNull(first.revocation.status())

        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        first.revocation.begin(RevocationScope.FULL, emptyList())
        val newRecoveryKey = first.revocation.newRecoveryKey()
        first.revocation.continueRevocation(token)
        assertNotEquals(oldMaster, keyOf(server.master))
        assertNotEquals(oldKeyId, server.defaultKeyId)
        // 古いリカバリーキーでは開けず、新しいリカバリーキーで開ける。
        assertFailsWith<ZooCryptoException.KeySlotUnlock> {
            first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        }
        first.revocation.openSecretStorage(UnlockMethod.Recovery(newRecoveryKey))
    }

    @Test
    fun fullRevocationNeedsANewRecoveryKey() = runBlocking<Unit> {
        val (first, recoveryKey) = world.setUp()
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        first.revocation.begin(RevocationScope.FULL, emptyList())
        // 新しい SSSS 鍵のスロットがなければ、SSSS の要求を作らない（サーバーに何も送らない）。
        val before = server.requests.size
        assertFailsWith<ZooCryptoException> { first.revocation.continueRevocation(token) }
        assertTrue(server.requests.drop(before).none { it.method != "GET" })
    }

    @Test
    fun pendingRevocationIsNotReplacedSilently() = runBlocking<Unit> {
        val (first, recoveryKey) = world.setUp()
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        val begun = first.revocation.begin(RevocationScope.DEVICE_ONLY, emptyList())
        val before = server.requests.size
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        assertFailsWith<RevocationException.Pending> { first.revocation.begin(RevocationScope.FULL, emptyList()) }
        assertTrue(server.requests.drop(before).none { it.method != "GET" && !it.path.endsWith("/used") })
        assertEquals(begun.revocationId, first.revocation.status()!!.revocationId)

        // 捨てる失効を指定したら、その失効の記録だけを捨てる。
        assertFailsWith<RevocationException.Changed> { first.revocation.abandon("another") }
        assertEquals(begun.revocationId, first.revocation.status()!!.revocationId)
        first.revocation.abandon(begun.revocationId)
        assertNull(first.revocation.status())
        assertFalse(first.store.hasPendingRevocation)
    }

    @Test
    fun replacedRevocationSavesNothing() = runBlocking<Unit> {
        val (first, recoveryKey) = world.setUp()
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        val begun = first.revocation.begin(RevocationScope.DEVICE_ONLY, emptyList())
        // 画面を離れて、失効を捨ててから別の失効を始め直した。
        first.revocation.abandon()
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        first.revocation.begin(RevocationScope.DEVICE_ONLY, emptyList())
        val before = server.requests.size
        assertFailsWith<RevocationException.Changed> {
            first.revocation.continueRevocation(token, expectedId = begun.revocationId)
        }
        assertTrue(server.requests.drop(before).none { it.method != "GET" })
        assertFailsWith<RevocationException.Changed> { first.revocation.resume(begun.revocationId) }
        // 今の失効は捨てていない。
        assertTrue(first.store.hasPendingRevocation)
    }

    @Test
    fun malformedUploadKeyStopsTheMigrationWithoutDeletingTheOldVersion() = runBlocking<Unit> {
        val (first, recoveryKey) = world.setUp()
        first.revocation.openSecretStorage(UnlockMethod.Recovery(recoveryKey))
        first.revocation.begin(RevocationScope.DEVICE_ONLY, emptyList())
        server.after = Pair<String, suspend () -> Unit>("POST /api/e2ee/key_backup/versions", { server.versions.last().uploadKey = JsonPrimitive(42) })
        val failure = assertFailsWith<IllegalStateException> { first.revocation.continueRevocation(token) }
        assertTrue("upload_key" in failure.message.orEmpty())
        assertEquals(listOf("1", "2"), server.liveVersions())
        assertTrue(server.sessions["2"].isNullOrEmpty())
        assertTrue(migrationPosts("2").isEmpty())

        // 鍵の形に戻れば、続きから証明付きで移す。
        server.versions.last().uploadKey = JsonPrimitive(BackupProofTestKit.UPLOAD_KEY)
        first.revocation.continueRevocation(token)
        assertEquals(listOf("2"), server.liveVersions())
        assertEquals(3, server.sessions["2"]?.size)
    }

    @Test
    fun authzRevocationUrlsUseTheIssuerOrigin() {
        assertEquals(
            AuthzRevocationUrls("https://account.zli.works/settings/security", "https://account.zli.works/settings/services"),
            AuthzRevocationUrls.fromIssuer("https://account.zli.works/api/auth"),
        )
        assertNull(AuthzRevocationUrls.fromIssuer("not a url"))
        assertEquals(listOf(AuthzRevocationStep.REVOKE_SESSIONS, AuthzRevocationStep.DISCONNECT), AuthzRevocationStep.BEFORE)
    }
}
