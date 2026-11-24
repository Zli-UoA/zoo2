package zoo.crypto

import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlin.time.Duration.Companion.milliseconds
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import zoo.SAS_EMOJI
import zoo.sasEmojiItems
import zoo.crypto.ffi.SasPhase
import zoo.crypto.ffi.VerificationMethod

/**
 * 他のユーザーの絵文字 SAS 検証と QR コードによる検証（#368 の Android の分）。Web の `sasController.test.ts` と同じ
 * 観点を、2 人の実際の暗号ストア（zoo-crypto の UniFFI）と状態を持つ模擬サーバー（[SasWorld]）で確かめる。
 * 検証のメッセージは Olm で暗号化して to-device で届け、[CryptoStore.processToDevice] から [SasController] に渡る。
 */
class SasVerificationTest {
    private val world = SasWorld()

    @AfterTest
    fun cleanUp() = world.close()

    /** Alice が Bob に検証を求め、Bob が受け入れて、双方が方式を選ぶ段階まで進める。 */
    private fun chooseMethod(alice: SasWorld.SasDevice, bob: SasWorld.SasDevice): String = runBlocking {
        val txn = alice.sas.request(BOB)
        assertEquals(SasPhase.REQUESTED, alice.entry(txn).view.phase)
        assertTrue(alice.entry(txn).view.weRequested)

        // 検証のメッセージは processToDevice から渡り、受信用の鍵には数えない。
        val received = bob.receive()
        assertEquals(1, received.processed.size)
        assertEquals(0, received.roomKeys)
        assertEquals(0, received.dropped)
        assertEquals(1, received.verifications.size)
        val incoming = bob.entry(txn).view
        assertEquals(SasPhase.INCOMING, incoming.phase)
        assertEquals(ALICE, incoming.theirUserId)
        assertEquals(alice.store.deviceId, incoming.theirDeviceId)
        assertFalse(incoming.weRequested)

        bob.sas.accept(txn)
        alice.receive()
        assertEquals(SasPhase.CHOOSE_METHOD, alice.entry(txn).view.phase)
        assertEquals(SasPhase.CHOOSE_METHOD, bob.entry(txn).view.phase)
        txn
    }

    /** 絵文字を比べる段階まで進める。 */
    private fun compare(alice: SasWorld.SasDevice, bob: SasWorld.SasDevice): String = runBlocking {
        val txn = chooseMethod(alice, bob)
        assertTrue(alice.entry(txn).view.canStartSas)
        alice.sas.startSas(txn)
        assertEquals(VerificationMethod.SAS, alice.entry(txn).view.method)
        bob.receive() // start → accept
        alice.receive() // accept → key
        bob.receive() // key → key
        alice.receive() // key
        assertEquals(SasPhase.COMPARE, alice.entry(txn).view.phase)
        assertEquals(SasPhase.COMPARE, bob.entry(txn).view.phase)
        txn
    }

    private fun masterKey(user: String): String =
        world.setups.getValue(user).keys.getValue("master_key").jsonObject.getValue("keys").jsonObject.values.single().jsonPrimitive.content

    @Test
    fun emojiSasVerifiesBothUsersAndSavesTheReplacePinAndSignature() = runBlocking<Unit> {
        val alice = world.device(ALICE, "alice")
        val bob = world.device(BOB, "bob")
        val txn = compare(alice, bob)

        // 双方が同じ絵文字を見る。
        val emoji = assertNotNull(alice.entry(txn).view.emoji)
        assertEquals(7, emoji.size)
        assertContentEquals(emoji, bob.entry(txn).view.emoji)
        val items = sasEmojiItems(emoji) { it }
        assertEquals(7, items.size)
        assertTrue(items.all { it.emoji == SAS_EMOJI[it.index] && it.label == "e2ee.sas.emoji.${it.index}" })

        alice.sas.confirm(txn, CompletableDeferred("alice-reauth"))
        assertEquals(SasPhase.CONFIRMED, alice.entry(txn).view.phase)
        bob.receive() // mac
        bob.sas.confirm(txn, CompletableDeferred("bob-reauth")) // mac と done を送る
        alice.receive() // mac → done、done
        bob.receive() // done

        val aliceDone = alice.await(txn) { it.save == SasSaveState.SAVED }
        val bobDone = bob.await(txn) { it.save == SasSaveState.SAVED }
        assertEquals(SasPhase.DONE, aliceDone.view.phase)
        assertEquals(SasPhase.DONE, bobDone.view.phase)
        assertEquals(masterKey(BOB), aliceDone.view.verifiedMasterKey)
        assertEquals(masterKey(ALICE), bobDone.view.verifiedMasterKey)
        assertNull(aliceDone.error)
        assertNull(bobDone.error)

        // `replace: true` のピンと、相手のマスター鍵への署名を、再認証の ID トークンを付けて保存した。
        for ((user, target, token) in listOf(Triple(ALICE, BOB, "alice-reauth"), Triple(BOB, ALICE, "bob-reauth"))) {
            val pin = world.pinUploads.single { it.caller == user }
            assertTrue(pin.replace)
            assertEquals(token, pin.reauth)
            val body = pin.pins.single().jsonObject
            assertEquals(target, body.getValue("target_user_id").jsonPrimitive.content)
            assertEquals(masterKey(target), body.getValue("master_key").jsonPrimitive.content)
            assertTrue(body.getValue("signatures") is JsonObject)
            val signature = world.signatureUploads.single { it.caller == user }
            assertEquals(token, signature.reauth)
            assertEquals(target, signature.signatures.single().jsonObject.getValue("target_user_id").jsonPrimitive.content)
        }

        // 保存できたので、記録の登録待ちを外した（相手のマスター鍵を、検証した鍵として記録した）。
        val record = assertNotNull(alice.store.trustStore.record(BOB))
        assertEquals(masterKey(BOB), record.masterKey)
        assertFalse(record.pinPending)
        assertFalse(assertNotNull(bob.store.trustStore.record(ALICE)).pinPending)
    }

    @Test
    fun qrCodeShowAndScanVerifiesBothUsers() = runBlocking<Unit> {
        val alice = world.device(ALICE, "alice")
        val bob = world.device(BOB, "bob")
        val txn = chooseMethod(alice, bob)

        val qr = assertNotNull(alice.entry(txn).view.qrCode)
        assertContentEquals("MATRIX".encodeToByteArray(), qr.copyOfRange(0, 6))
        assertTrue(bob.entry(txn).view.canScanQrCode)

        // 形の正しくない QR は、取りやめずに読み直す。
        assertEquals(QrScanResult.RETRY, bob.sas.scanQr(txn, "MATRIX".encodeToByteArray()))
        assertEquals(SasPhase.CHOOSE_METHOD, bob.entry(txn).view.phase)

        assertEquals(QrScanResult.ACCEPTED, bob.sas.scanQr(txn, qr))
        assertEquals(SasPhase.QR_SCANNED, bob.entry(txn).view.phase)
        assertEquals(VerificationMethod.QR_CODE, bob.entry(txn).view.method)

        alice.receive() // start（m.reciprocate.v1）
        assertEquals(SasPhase.QR_CONFIRM, alice.entry(txn).view.phase)
        alice.sas.confirmQr(txn, CompletableDeferred("alice-reauth"))
        val aliceDone = alice.await(txn) { it.save == SasSaveState.SAVED }
        assertEquals(masterKey(BOB), aliceDone.view.verifiedMasterKey)

        bob.receive() // done → done
        // 読み取った側は、まだ再認証していない。「ログインして保存」で保存する。
        assertEquals(SasPhase.DONE, bob.await(txn) { it.save == SasSaveState.LOGIN }.view.phase)
        assertTrue(world.pinUploads.none { it.caller == BOB })
        bob.sas.retrySave(txn, CompletableDeferred("bob-reauth"))
        assertEquals(SasSaveState.SAVED, bob.entry(txn).save)
        assertEquals("bob-reauth", world.pinUploads.single { it.caller == BOB }.reauth)
        assertFalse(assertNotNull(bob.store.trustStore.record(ALICE)).pinPending)
        alice.receive() // Bob の done（もう終わっている）
        assertEquals(SasPhase.DONE, alice.entry(txn).view.phase)
    }

    @Test
    fun mismatchCancelsOnBothSides() = runBlocking<Unit> {
        val alice = world.device(ALICE, "alice")
        val bob = world.device(BOB, "bob")
        val txn = compare(alice, bob)

        alice.sas.mismatch(txn)
        val mine = alice.entry(txn).view
        assertEquals(SasPhase.CANCELLED, mine.phase)
        assertEquals("m.mismatched_sas", mine.cancel?.code)
        assertEquals(true, mine.cancel?.byUs)

        bob.receive()
        val theirs = bob.entry(txn).view
        assertEquals(SasPhase.CANCELLED, theirs.phase)
        assertEquals("m.mismatched_sas", theirs.cancel?.code)
        assertEquals(false, theirs.cancel?.byUs)
        assertNull(theirs.verifiedMasterKey)

        // 取りやめたあとは確かめられない。保存もしない。
        bob.sas.confirm(txn, CompletableDeferred("bob-reauth"))
        assertEquals(SasErrorCode.FAILED, bob.entry(txn).error)
        assertTrue(world.pinUploads.isEmpty())

        // 画面を閉じたら忘れる。
        bob.sas.dismiss(txn)
        assertTrue(bob.sas.entries.value.isEmpty())
    }

    @Test
    fun expiredVerificationIsCancelledWithTimeoutOnBothSides() = runBlocking<Unit> {
        // Alice は期限切れを裏で確かめる（短い間隔）。Bob はテストから確かめる。
        val alice = world.device(ALICE, "alice", expireInterval = 20.milliseconds)
        val bob = world.device(BOB, "bob")
        val txn = alice.sas.request(BOB)
        bob.receive()
        assertEquals(SasPhase.INCOMING, bob.entry(txn).view.phase)

        // 期限の前は取りやめない。
        world.now += 60_000
        bob.sas.expire()
        assertEquals(SasPhase.INCOMING, bob.entry(txn).view.phase)

        world.now += 10 * 60 * 1000
        val expired = alice.await(txn) { it.view.phase == SasPhase.CANCELLED }
        assertEquals("m.timeout", expired.view.cancel?.code)
        assertEquals(true, expired.view.cancel?.byUs)

        // 表示は送信より先に変わるので、取りやめが Bob に届くまで待つ。
        bob.awaitInbox()
        bob.receive()
        assertEquals(SasPhase.CANCELLED, bob.entry(txn).view.phase)
        assertEquals("m.timeout", bob.entry(txn).view.cancel?.code)

        // 古い要求は受け付けない（捨てる。取りやめも送らない）。
        val stale = alice.sas.request(BOB)
        world.now += 11 * 60 * 1000
        bob.receive()
        assertTrue(bob.sas.entries.value.none { it.view.transactionId == stale })
    }

    @Test
    fun keysQueryFailureHoldsTheMessageAndRetriesOnTheNextCheck() = runBlocking<Unit> {
        val alice = world.device(ALICE, "alice")
        val bob = world.device(BOB, "bob")
        val txn = alice.sas.request(BOB)
        world.failKeysQuery = 1
        bob.receive()
        // 送信ユーザーの鍵を取れなかった要求は、捨てずに残す。
        assertTrue(bob.sas.entries.value.isEmpty())
        bob.sas.expire()
        assertEquals(SasPhase.INCOMING, bob.entry(txn).view.phase)
        assertNull(bob.entry(txn).error)
    }

    @Test
    fun requestWhileLockedFails() = runBlocking<Unit> {
        world.device(BOB, "bob")
        // 初回設定も解錠もしていない端末。
        val locked = world.device(ALICE, "alice", setUp = false)
        val failure = assertFailsWith<SasException> { locked.sas.request(BOB) }
        assertEquals(SasException.NEEDS_UNLOCK, failure.code)
        assertTrue(locked.sas.entries.value.isEmpty())

        // 解錠し直しが要る（secrets_revision を消した）端末も求められない。
        val carol = world.device(CAROL, "carol")
        carol.store.forgetSecretsRevision(assertNotNull(carol.store.secretsRevision))
        assertEquals(SasException.NEEDS_UNLOCK, assertFailsWith<SasException> { carol.sas.request(BOB) }.code)
    }

    private companion object {
        const val ALICE = "00000000-0000-4000-8000-00000000a11c"
        const val BOB = "00000000-0000-4000-8000-000000000b0b"
        const val CAROL = "00000000-0000-4000-8000-0000000ca201"
    }
}
