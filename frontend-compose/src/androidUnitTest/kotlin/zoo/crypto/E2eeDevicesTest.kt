package zoo.crypto

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject

/** 端末一覧の「未署名」（Web の `DevicesSettings` と同じく、今の自己署名鍵の署名があるかだけを見る）。 */
class E2eeDevicesTest {
    private val me = "u"

    private fun query(signatures: String = "[]", embedded: String = "{}", selfSigning: String = """{"keys":{"ed25519:SSK":"SSK"}}""") =
        Json.parseToJsonElement("""{
            "self_signing_keys": {"u": $selfSigning},
            "device_keys": {"u": {"A": {"signatures": {"u": $embedded}}, "B": {}, "C": {}}},
            "signatures": $signatures
        }""").jsonObject

    @Test
    fun devicesWithoutTheCurrentSelfSigningSignatureAreUnsigned() {
        val rows = """[
            {"signer_user_id":"u","target_user_id":"u","target_key_id":"B","signer_key_id":"ed25519:SSK","signature":"s"},
            {"signer_user_id":"u","target_user_id":"u","target_key_id":"C","signer_key_id":"ed25519:OLD","signature":"s"},
            {"signer_user_id":"x","target_user_id":"u","target_key_id":"C","signer_key_id":"ed25519:SSK","signature":"s"}
        ]"""
        val unsigned = unsignedDevices(query(rows, """{"ed25519:SSK":"sig"}"""), me, listOf("A", "B", "C"))
        assertEquals(setOf("C"), unsigned)
    }

    @Test
    fun withoutASelfSigningKeyEveryDeviceIsUnsigned() {
        assertEquals(setOf("A", "B"), unsignedDevices(query(selfSigning = "{}"), me, listOf("A", "B")))
    }
}
