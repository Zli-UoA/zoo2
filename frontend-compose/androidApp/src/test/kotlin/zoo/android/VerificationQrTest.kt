package zoo.android

import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertNull

/** 検証の QR の描画と読み取り（Web の `qrCode.ts` と同じ、バイトモードのバイト列）。 */
class VerificationQrTest {
    /** モジュールを、1 モジュール 4 画素、周りに 4 モジュールの余白を付けた輝度の画像にする。 */
    private fun render(modules: List<BooleanArray>, scale: Int = 4, quiet: Int = 4): Triple<ByteArray, Int, Int> {
        val size = (modules.size + quiet * 2) * scale
        val pixels = ByteArray(size * size) { 0xff.toByte() }
        modules.forEachIndexed { y, row ->
            row.forEachIndexed { x, dark ->
                if (dark) for (dy in 0 until scale) for (dx in 0 until scale) {
                    pixels[((y + quiet) * scale + dy) * size + (x + quiet) * scale + dx] = 0
                }
            }
        }
        return Triple(pixels, size, size)
    }

    @Test
    fun binaryPayloadsRoundTrip() {
        // MATRIX、版 2、方式 0、transaction_id の長さ、鍵と秘密（0〜255 のすべての値を含める）。
        val payload = "MATRIX".toByteArray() + byteArrayOf(0x02, 0x00, 0x00, 0x04) + "txn1".toByteArray() +
            ByteArray(80) { it.toByte() } + ByteArray(176) { (it + 80).toByte() }
        val (pixels, width, height) = render(qrModules(payload))
        assertContentEquals(payload, decodeQr(pixels, width, height))
    }

    @Test
    fun anImageWithoutACodeGivesNull() {
        assertNull(decodeQr(ByteArray(64 * 64) { 0xff.toByte() }, 64, 64))
    }
}
