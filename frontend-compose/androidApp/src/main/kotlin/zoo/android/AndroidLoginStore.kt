package zoo.android

import android.content.Context
import android.content.SharedPreferences
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import java.util.Base64
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/** Only authenticated ciphertext enters private preferences; backups are disabled by the manifest. */
internal class AndroidLoginStore(
    private val preferences: SharedPreferences,
    private val readKey: () -> SecretKey?,
    private val createKey: () -> SecretKey,
    private val deleteKey: () -> Unit,
) {
    constructor(context: Context) : this(
        context.getSharedPreferences("saved_login", Context.MODE_PRIVATE),
        AndroidLoginKeys::read,
        AndroidLoginKeys::create,
        AndroidLoginKeys::delete,
    )

    // All instances belong to the main app process. A new Activity/login supersedes
    // the old writer, including a refresh finishing in a NonCancellable context.
    private companion object {
        val lock = Any()
        var generation = 0L
    }

    val hasSession: Boolean get() = synchronized(lock) { preferences.contains("session") }

    fun beginSession(expected: SavedLogin? = null): Long = synchronized(lock) {
        if (expected != null) {
            val current = load()
            if (current == null || current.connection != expected.connection ||
                current.refreshToken != expected.refreshToken
            ) throw LoginFailure("保存した認証情報が変更されました。自動ログインを再試行してください")
        }
        ++generation
    }

    private fun checkOwner(owner: Long) {
        if (owner != generation) throw LoginFailure("別のログインが開始されました。ログイン画面からやり直してください")
    }

    fun load(): SavedLogin? = synchronized(lock) {
        val encoded = preferences.getString("session", null) ?: return@synchronized null
        try {
            // Reading must never generate a replacement key for unreadable ciphertext.
            val key = readKey() ?: error("Missing login key")
            SavedLogin.decode(LoginCipher(key).decrypt(Base64.getDecoder().decode(encoded)))
        } catch (_: Exception) {
            clear()
            null
        }
    }

    /** Fail before the HTTP request if removing the old token cannot be committed. */
    fun consume(owner: Long) = synchronized(lock) {
        checkOwner(owner)
        if (!preferences.edit().remove("session").commit()) {
            throw LoginFailure("更新前の認証情報を削除できませんでした。端末の空き容量を確認してください")
        }
    }

    fun save(login: SavedLogin, owner: Long) = synchronized(lock) {
        checkOwner(owner)
        try {
            val bytes = LoginCipher(readKey() ?: createKey()).encrypt(login.encode())
            check(preferences.edit().putString("session", Base64.getEncoder().encodeToString(bytes)).commit())
        } catch (_: Exception) {
            throw LoginFailure("認証情報を保存できませんでした。端末の空き容量とロック設定を確認してください")
        }
    }

    fun clear(owner: Long? = null) = synchronized(lock) {
        if (owner != null && owner != generation) return@synchronized
        if (owner == null) generation++
        try {
            val removed = preferences.edit().remove("session").commit()
            deleteKey()
            check(removed)
        } catch (_: Exception) {
            throw LoginFailure("保存した認証情報を削除できませんでした。端末の空き容量とロック設定を確認してください")
        }
    }
}

private object AndroidLoginKeys {
    private const val ALIAS = "zoo-login-v1"
    private val keys get() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    fun read(): SecretKey? = keys.getKey(ALIAS, null) as? SecretKey
    fun delete() = keys.deleteEntry(ALIAS)
    fun create(): SecretKey = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
        init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        generateKey()
    }
}
