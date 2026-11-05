package zoo.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/** Only authenticated ciphertext enters private preferences; backups are disabled by the manifest. */
internal class AndroidLoginStore(context: Context) {
    private val preferences = context.getSharedPreferences("saved_login", Context.MODE_PRIVATE)
    private val alias = "zoo-login-v1"
    private val keys get() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    val hasSession: Boolean get() = preferences.contains("session")

    private fun key(): SecretKey {
        (keys.getKey(alias, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(
                KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256)
                    .build()
            )
            generateKey()
        }
    }

    fun load(): SavedLogin? {
        val encoded = preferences.getString("session", null) ?: return null
        return try {
            // Reading must never generate a replacement key for unreadable ciphertext.
            val key = keys.getKey(alias, null) as? SecretKey ?: error("Missing login key")
            SavedLogin.decode(LoginCipher(key).decrypt(Base64.decode(encoded, Base64.NO_WRAP)))
        } catch (_: Exception) {
            clear()
            null
        }
    }

    fun save(login: SavedLogin) {
        try {
            val bytes = LoginCipher(key()).encrypt(login.encode())
            check(preferences.edit().putString("session", Base64.encodeToString(bytes, Base64.NO_WRAP)).commit())
        } catch (_: Exception) {
            throw LoginFailure("認証情報を保存できませんでした。端末の空き容量とロック設定を確認してください")
        }
    }

    fun clear() {
        try {
            val removed = preferences.edit().remove("session").commit()
            keys.deleteEntry(alias)
            check(removed)
        } catch (_: Exception) {
            throw LoginFailure("保存した認証情報を削除できませんでした。端末の空き容量とロック設定を確認してください")
        }
    }
}
