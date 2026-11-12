package zoo.crypto

import zoo.crypto.ffi.DeviceAccount
import java.io.File
import java.nio.file.Files
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse

/**
 * 保存鍵を包んで置く処理。JVM では Android Keystore を使えないので、同じ AES-256-GCM の
 * ソフトウェアの鍵で包む（Android Keystore の鍵との違いは、鍵を端末の外に出せるかだけ）。
 */
class StorageKeyStoreTest {
    private val directory: File = Files.createTempDirectory("zoo-storage-key").toFile()
    private val file = File(directory, "storage-key")

    @AfterTest
    fun cleanUp() {
        directory.deleteRecursively()
    }

    @Test
    fun createsOnceAndLoadsTheSameKey() {
        val wrapping = softwareKey()
        val created = StorageKeyStore(file, { wrapping }).loadOrCreate()
        assertEquals(32, created.size)
        // 1 + IV 12 + 鍵 32 + タグ 16
        assertEquals(61, file.length())
        assertContentEquals(created, StorageKeyStore(file, { wrapping }).loadOrCreate())
    }

    @Test
    fun storageKeyOpensTheDevicePickle() {
        val wrapping = softwareKey()
        val pickle = DeviceAccount(USER, DEVICE).use { account ->
            account.pickle(StorageKeyStore(file, { wrapping }).loadOrCreate())
        }
        val storageKey = StorageKeyStore(file, { wrapping }).loadOrCreate()
        DeviceAccount.fromPickle(pickle, storageKey, USER, DEVICE).close()
    }

    @Test
    fun failsWithAnotherWrappingKey() {
        StorageKeyStore(file, { softwareKey() }).loadOrCreate()
        val other = softwareKey()
        assertFailsWith<StorageKeyUnavailableException> { StorageKeyStore(file, { other }).loadOrCreate() }
    }

    @Test
    fun failsWhenTampered() {
        val wrapping = softwareKey()
        StorageKeyStore(file, { wrapping }).loadOrCreate()
        val bytes = file.readBytes()
        bytes[bytes.size - 1] = (bytes[bytes.size - 1].toInt() xor 1).toByte()
        file.writeBytes(bytes)
        assertFailsWith<StorageKeyUnavailableException> { StorageKeyStore(file, { wrapping }).loadOrCreate() }

        file.writeBytes(bytes.copyOf(10))
        assertFailsWith<StorageKeyUnavailableException> { StorageKeyStore(file, { wrapping }).loadOrCreate() }
    }

    @Test
    fun deleteRemovesTheFile() {
        val store = StorageKeyStore(file, { softwareKey() })
        store.loadOrCreate()
        store.delete()
        assertFalse(file.exists())
    }

    private fun softwareKey(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    private companion object {
        const val USER = "00000000-0000-4000-8000-00000000000a"
        const val DEVICE = "00000000-0000-4000-8000-0000000000a1"
    }
}
