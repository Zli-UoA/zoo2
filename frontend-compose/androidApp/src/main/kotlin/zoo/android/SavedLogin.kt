package zoo.android

import io.ktor.http.*
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import zoo.*

internal data class LoginConnection(
    val base: String,
    val issuer: String,
    val client: String,
    val resource: String,
) {
    fun validate() {
        val server = Url(base)
        require(
            server.protocol == URLProtocol.HTTPS &&
                server.user.isNullOrEmpty() && server.password.isNullOrEmpty() &&
                server.encodedPath in listOf("", "/") &&
                server.parameters.isEmpty() && server.fragment.isEmpty()
        ) { "サーバーには HTTPS の origin URL を指定してください" }
        val provider = Url(issuer)
        require(
            provider.protocol == URLProtocol.HTTPS &&
                provider.user.isNullOrEmpty() && provider.password.isNullOrEmpty() &&
                provider.parameters.isEmpty() && provider.fragment.isEmpty()
        ) { "認証先は HTTPS の issuer URL を指定してください" }
        require(client.isNotBlank()) { "Public client ID を指定してください" }
    }
}

/** The encrypted record binds the refresh token to the original API and OIDC settings. */
internal class SavedLogin(val connection: LoginConnection, val refreshToken: String) {
    fun encode(): String = obj(
        "base" to connection.base, "issuer" to connection.issuer,
        "client" to connection.client, "resource" to connection.resource,
        "refreshToken" to refreshToken,
    ).toString()

    companion object {
        fun decode(raw: String): SavedLogin {
            val row = json.parseToJsonElement(raw) as kotlinx.serialization.json.JsonObject
            val connection = LoginConnection(row.text("base"), row.text("issuer"), row.text("client"), row.text("resource"))
            connection.validate()
            require(row.text("refreshToken").isNotBlank())
            return SavedLogin(connection, row.text("refreshToken"))
        }
    }
}

/** Versioned AES-GCM envelope; a new IV is generated for every write. */
internal class LoginCipher(private val key: SecretKey) {
    private val aad = "works.zli.zoo.login.v1".toByteArray(Charsets.UTF_8)

    fun encrypt(value: String): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key)
        cipher.updateAAD(aad)
        require(cipher.iv.size == 12)
        return byteArrayOf(1) + cipher.iv + cipher.doFinal(value.toByteArray(Charsets.UTF_8))
    }

    fun decrypt(value: ByteArray): String {
        require(value.size >= 29 && value[0] == 1.toByte())
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, value.copyOfRange(1, 13)))
        cipher.updateAAD(aad)
        return String(cipher.doFinal(value.copyOfRange(13, value.size)), Charsets.UTF_8)
    }
}
