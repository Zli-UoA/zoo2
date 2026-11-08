package zoo.android

import javax.crypto.AEADBadTagException
import javax.crypto.KeyGenerator
import kotlin.test.*

class SavedLoginTest {
    private fun newCipher() = LoginCipher(KeyGenerator.getInstance("AES").apply { init(256) }.generateKey())
    private val connection = LoginConnection("https://zoo.example", "https://id.example/auth", "client", "https://zoo.example/api")

    @Test
    fun encryptedSessionRoundTripKeepsOriginalConnectionAndRefreshToken() {
        val cipher = newCipher()
        val original = SavedLogin(connection, "test-refresh-token")
        val encrypted = cipher.encrypt(original.encode())
        assertFalse(String(encrypted, Charsets.ISO_8859_1).contains("test-refresh-token"))
        val restored = SavedLogin.decode(cipher.decrypt(encrypted))
        assertEquals(connection, restored.connection)
        assertEquals("test-refresh-token", restored.refreshToken)
    }

    @Test
    fun eachSaveUsesDifferentCiphertext() {
        val cipher = newCipher()
        assertFalse(cipher.encrypt("same session").contentEquals(cipher.encrypt("same session")))
    }

    @Test
    fun tamperingOrAnotherKeyCannotRestoreSession() {
        val cipher = newCipher()
        val encrypted = cipher.encrypt("session")
        assertFailsWith<AEADBadTagException> { newCipher().decrypt(encrypted) }
        encrypted[encrypted.lastIndex] = (encrypted.last().toInt() xor 1).toByte()
        assertFailsWith<AEADBadTagException> { cipher.decrypt(encrypted) }
    }

    @Test
    fun rejectsTruncatedOrUnsupportedEnvelope() {
        val cipher = newCipher()
        assertFailsWith<IllegalArgumentException> { cipher.decrypt(byteArrayOf(1)) }
        val unsupported = cipher.encrypt("session").also { it[0] = 2 }
        assertFailsWith<IllegalArgumentException> { cipher.decrypt(unsupported) }
    }

    @Test
    fun savedConnectionCannotSendCredentialsToAnInsecureOrNonOriginApi() {
        for (base in listOf("http://zoo.example", "https://user:password@zoo.example", "https://zoo.example/api", "https://zoo.example?other=server")) {
            assertFailsWith<IllegalArgumentException> { SavedLogin.decode(SavedLogin(connection.copy(base = base), "token").encode()) }
        }
        assertFailsWith<IllegalArgumentException> { SavedLogin.decode(SavedLogin(connection.copy(issuer = "http://id.example"), "token").encode()) }
        assertFailsWith<IllegalArgumentException> { SavedLogin.decode(SavedLogin(connection, "").encode()) }
    }
}
