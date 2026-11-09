package zoo

import kotlin.test.*

class ImageMetadataTest {
    @Test
    fun decoderMatchesTheWebBlurhashReference() {
        val pixels = decodeBlurhash("LEHV6nWB2yk8pyo0adR*.7kCMdnj", 2)!!
        val expected = intArrayOf(0xff87a4b1.toInt(), 0xffb5b4ab.toInt(), 0xff7894a2.toInt(), 0xff9e7d6c.toInt())
        assertContentEquals(expected, pixels)
        assertContentEquals(IntArray(4) { 0xff000000.toInt() }, decodeBlurhash("000000", 2))
    }

    @Test
    fun malformedHashesAndInvalidDimensionsFallBack() {
        for (hash in listOf("", "bad", "00000!", "~00000", "é00000", "0".repeat(200))) {
            assertNull(decodeBlurhash(hash))
        }
        assertNull(decodeBlurhash("000000", 100000))
        assertEquals(1.5f, imageAspectRatio(obj("width" to "600", "height" to "400")))
        assertEquals(4f / 3f, imageAspectRatio(obj("width" to "0", "height" to "-1")))
    }
}
