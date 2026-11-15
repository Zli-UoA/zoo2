package zoo.crypto

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import java.io.File
import java.io.IOException
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * 暗号ストアの保存鍵（32 バイト）を包む鍵。docs/e2ee.md の「暗号ストア」の【決定】で、
 * Android は Android Keystore の鍵で包む。
 *
 * 単体テスト（JVM）では Android Keystore を使えないので、ソフトウェアの AES 鍵に差し替える。
 */
fun interface WrappingKeyProvider {
    /** AES-256-GCM の鍵。なければ作る。 */
    fun wrappingKey(): SecretKey
}

/**
 * Android Keystore の AES-256-GCM の鍵。鍵は端末の外に出せず、アプリを消すと消える。
 *
 * 別の端末へのバックアップや復元では鍵が移らないので、包んだ保存鍵は開けなくなる
 * （[StorageKeyUnavailableException]）。そのときは暗号ストアを作り直し、SSSS で解錠し直す（#359）。
 */
class AndroidKeystoreWrappingKey(
    private val alias: String = DEFAULT_ALIAS,
) : WrappingKeyProvider {
    override fun wrappingKey(): SecretKey {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (keyStore.getKey(alias, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                // IV は Keystore に作らせる（呼び出し側が IV を選べないようにする）。
                .setRandomizedEncryptionRequired(true)
                .build(),
        )
        return generator.generateKey()
    }

    /** 包む鍵を消す。ログアウトで暗号ストアを消すとき（#359）に使う。 */
    fun delete() {
        KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }.deleteEntry(alias)
    }

    companion object {
        const val ANDROID_KEYSTORE = "AndroidKeyStore"
        const val DEFAULT_ALIAS = "zoo-e2ee-storage-key-wrap-v1"
    }
}

/**
 * 包んだ保存鍵をもう開けない（包む鍵が消えた・失効した、別の端末に移った、ファイルが壊れた・改ざんされた）。
 * 再試行しても開けないので、呼び出し側は暗号ストアを作り直す。
 *
 * Keystore の一時的な失敗（`KeyStoreException`、`UnrecoverableKeyException` など）は、これにせず
 * そのまま投げる。暗号ストアを作り直すと、Olm セッションや、まだバックアップしていない
 * Megolm の受信用セッションが消えるため。
 */
class StorageKeyUnavailableException(message: String, cause: Throwable? = null) : Exception(message, cause)

/**
 * 暗号ストアの保存鍵を、[WrappingKeyProvider] の鍵で包んで [file] に置く。
 *
 * 保存鍵は zoo-crypto の `DeviceAccount.pickle` などに渡す 32 バイトで、vodozemac の pickle key は
 * zoo-crypto が保存のたびにここから導く。ファイルの形式は次のとおり（版 1）。
 *
 * ```
 * 0x01 ‖ IV（12 バイト） ‖ AES-256-GCM(保存鍵) と タグ（32 + 16 バイト）
 * ```
 *
 * AAD は `zoo e2ee storage key v1`。ファイルは暗号ストアごとに置く（[CryptoStore]、#359）。
 */
class StorageKeyStore(
    private val file: File,
    private val wrappingKey: WrappingKeyProvider,
    private val random: SecureRandom = SecureRandom(),
) {
    /**
     * 保存鍵を読む。なければ作って保存する。返した配列は使い終わったら呼び出し側で 0 で埋める。
     *
     * もう開けないときは [StorageKeyUnavailableException] を投げる。それ以外の例外
     * （Keystore の一時的な失敗など）はそのまま投げるので、呼び出し側は暗号ストアを消さずに再試行する。
     */
    @Synchronized
    fun loadOrCreate(): ByteArray = if (file.exists()) load() else create()

    /**
     * 保存鍵を読む。ファイルがなければ `null`（作らない）。暗号ストアの状態はあるのに保存鍵がない
     * ときに、新しい鍵を作って取り違えないために使う。例外は [loadOrCreate] と同じ。
     */
    @Synchronized
    fun loadExisting(): ByteArray? = if (file.exists()) load() else null

    /** 保存鍵を消す（ログアウトで暗号ストアを消すとき）。包む鍵は消さない。 */
    @Synchronized
    fun delete() {
        if (file.exists() && !file.delete()) throw IOException("保存鍵のファイルを消せません: $file")
    }

    private fun load(): ByteArray {
        val bytes = file.readBytes()
        if (bytes.size != WRAPPED_SIZE || bytes[0] != VERSION) {
            throw StorageKeyUnavailableException("保存鍵のファイルの形式が正しくありません")
        }
        return try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, wrappingKey.wrappingKey(), GCMParameterSpec(TAG_BITS, bytes, 1, IV_SIZE))
            cipher.updateAAD(AAD)
            cipher.doFinal(bytes, 1 + IV_SIZE, bytes.size - 1 - IV_SIZE).also {
                check(it.size == KEY_SIZE)
            }
        } catch (e: AEADBadTagException) {
            // 包む鍵が違う（消えて作り直された、別の端末に移った）か、ファイルが改ざんされた。
            throw StorageKeyUnavailableException("保存鍵を開けません", e)
        } catch (e: KeyPermanentlyInvalidatedException) {
            throw StorageKeyUnavailableException("保存鍵を包む鍵が失効しました", e)
        }
    }

    private fun create(): ByteArray {
        val key = ByteArray(KEY_SIZE).also(random::nextBytes)
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, wrappingKey.wrappingKey())
        cipher.updateAAD(AAD)
        val sealed = cipher.doFinal(key)
        val iv = cipher.iv
        check(iv.size == IV_SIZE && sealed.size == KEY_SIZE + TAG_BITS / 8)
        val wrapped = byteArrayOf(VERSION) + iv + sealed
        // 内容とディレクトリーのエントリーを同期してから返す（この鍵で保存した state.json だけが残り、
        // 保存鍵を失うことがないように）。
        CryptoStore.writeAtomically(file, wrapped)
        return key
    }

    private companion object {
        const val VERSION: Byte = 1
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val KEY_SIZE = 32
        const val IV_SIZE = 12
        const val TAG_BITS = 128
        const val WRAPPED_SIZE = 1 + IV_SIZE + KEY_SIZE + TAG_BITS / 8
        val AAD = "zoo e2ee storage key v1".encodeToByteArray()
    }
}
