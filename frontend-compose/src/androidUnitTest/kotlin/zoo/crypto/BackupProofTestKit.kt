package zoo.crypto

import java.math.BigInteger
import java.security.KeyFactory
import java.security.spec.NamedParameterSpec
import java.security.spec.XECPrivateKeySpec
import java.security.spec.XECPublicKeySpec
import java.util.Base64
import javax.crypto.KeyAgreement
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * テストのサーバーが、鍵バックアップのアップロードの証明（docs/e2ee.md の「アップロードの証明」）を確かめる道具。
 * zoo-crypto を使わず、JDK の X25519（XDH）と HMAC-SHA256 の HKDF で別に計算する（Web の `backupProofTestKit.ts`
 * と同じ値）。サーバーの upload key の秘密鍵は 0x22 の 32 バイト（`crates/zoo-crypto` の照合用の値と同じ）。
 */
internal object BackupProofTestKit {
    /** upload key の公開鍵（0x22 の 32 バイトの秘密鍵から作ったもの。パディングなしの base64）。 */
    const val UPLOAD_KEY = "D6poTtKIZ7l/Smot7l34zpdOdrcBjj8iocTPJnhXDyA"
    /** 別の upload key の公開鍵（0x11 の 32 バイトの秘密鍵）。証明が合わない場合に使う。 */
    const val OTHER_UPLOAD_KEY = "e06Qm75//kTEZaIgA31gjuNYl9Me+XLwf3SJLLD3PxM"

    private const val INFO = "zoo e2ee megolm backup upload v1"

    private fun decode(value: String): ByteArray = Base64.getDecoder().decode(value.padEnd((value.length + 3) / 4 * 4, '='))

    private fun lengthPrefixed(vararg parts: String): ByteArray =
        parts.fold(ByteArray(0)) { out, part ->
            val bytes = part.toByteArray(Charsets.UTF_8)
            out + java.nio.ByteBuffer.allocate(4).putInt(bytes.size).array() + bytes
        }

    private fun hmac(key: ByteArray, data: ByteArray): ByteArray =
        Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec(key, "HmacSHA256")) }.doFinal(data)

    /** サーバーが作る証明（`X25519(upload key の秘密鍵, auth_data.public_key)` から HKDF）。 */
    fun expectedProof(backupPublicKey: String, userId: String, version: String): String {
        val factory = KeyFactory.getInstance("XDH")
        val private = factory.generatePrivate(XECPrivateKeySpec(NamedParameterSpec.X25519, ByteArray(32) { 0x22 }))
        val u = BigInteger(1, decode(backupPublicKey).reversedArray()).clearBit(255)
        val public = factory.generatePublic(XECPublicKeySpec(NamedParameterSpec.X25519, u))
        val shared = KeyAgreement.getInstance("XDH").run { init(private); doPhase(public, true); generateSecret() }
        val prk = hmac(ByteArray(32), shared)
        val okm = hmac(prk, lengthPrefixed(INFO, userId, version) + byteArrayOf(1))
        return Base64.getEncoder().withoutPadding().encodeToString(okm.copyOf(32))
    }
}
