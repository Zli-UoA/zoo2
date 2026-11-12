package zoo

import kotlin.test.*

class DmNamesTest {
    @Test
    fun plainAndEncryptedOneToOneDmsShareTheNameFormat() {
        assertEquals(listOf("u1", "u2"), oneToOneDmIds("dm:u1:u2"))
        // 暗号化 DM は edm: の名前（docs/e2ee.md）。
        assertEquals(listOf("u1", "u2"), oneToOneDmIds("edm:u1:u2"))
        assertNull(oneToOneDmIds("dm:group:abc"))
        assertNull(oneToOneDmIds("general"))
        assertNull(oneToOneDmIds("edmx:u1:u2"))
    }
}
