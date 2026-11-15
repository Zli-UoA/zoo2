package zoo

import kotlin.test.*
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

class EncryptedChannelsTest {
    private fun channel(id: String, encrypted: String?) = buildJsonObject {
        put("id", JsonPrimitive(id))
        if (encrypted != null) put("encrypted", JsonPrimitive(encrypted))
    }

    @Test
    fun onlyChannelsMarkedEncryptedBlockSending() {
        val rows = listOf(channel("plain", "f"), channel("secret", "t"), channel("old", null))
        // Electric は bool を "t" / "f" で配る。暗号化チャンネルには平文を送らない（docs/e2ee.md）。
        assertTrue(isEncryptedChannel(rows, "secret"))
        assertFalse(isEncryptedChannel(rows, "plain"))
        // 古いスキーマの行（encrypted がない）は平文のチャンネルとして扱う。
        assertFalse(isEncryptedChannel(rows, "old"))
        assertFalse(isEncryptedChannel(rows, "missing"))
    }
}
