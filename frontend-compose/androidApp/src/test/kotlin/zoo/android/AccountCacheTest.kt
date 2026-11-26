package zoo.android

import java.io.File
import kotlin.test.*
import org.junit.Rule
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 35], manifest = Config.NONE)
class AccountCacheTest {
    @get:Rule val temporary = TemporaryFolder()

    @Test
    fun snapshotsSurviveReopeningAndUseIndependentKeys() {
        val directory = File(temporary.root, "account")
        val cache = AccountCache(directory)
        assertNull(cache.read("messages"))
        cache.write("messages", "日本語の投稿")
        cache.write("channels", "[]")
        cache.write("messages", "新しい投稿")
        val reopened = AccountCache(directory)
        assertEquals("新しい投稿", reopened.read("messages"))
        assertEquals("[]", reopened.read("channels"))
    }

    @Test
    fun logoutPreventsAnAlreadyQueuedWriterFromRecreatingTheCache() {
        val directory = File(temporary.root, "account")
        val cache = AccountCache(directory)
        cache.write("messages", "private")
        cache.clear()
        cache.write("messages", "late private snapshot")
        assertNull(cache.read("messages"))
        assertFalse(directory.exists())
    }

    @Test
    fun interruptedWriteRecoversTheLastCommittedSnapshot() {
        val directory = File(temporary.root, "account")
        val cache = AccountCache(directory)
        cache.write("messages", "committed")
        val target = directory.listFiles()!!.single()
        // AtomicFile on API 26 uses a backup; recover it before reading the incomplete base.
        assertTrue(target.renameTo(File(target.path + ".bak")))
        target.writeText("partial")
        assertEquals("committed", cache.read("messages"))
    }
}
