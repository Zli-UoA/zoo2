package zoo.android

import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.EncodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ResultMetadataType
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import com.google.zxing.qrcode.encoder.Encoder

/**
 * 他のユーザーの検証の QR（docs/e2ee.md の「QR コードによる検証」、#567）の描画と読み取り。中身の作成と検証は
 * zoo-crypto（`SasView.qrCode`、`SasVerification.scanQrCode`）が行い、ここはバイト列と画像の変換だけを持つ
 * （Web の `qrCode.ts` と同じ。バイトモード、誤り訂正は M）。
 */

/** QR のモジュール（黒なら `true`）。1 文字が 1 バイトの ISO-8859-1 で、バイト列をそのままバイトモードに入れる。 */
internal fun qrModules(bytes: ByteArray): List<BooleanArray> {
    val text = String(bytes, Charsets.ISO_8859_1)
    val code = Encoder.encode(text, ErrorCorrectionLevel.M, mapOf(EncodeHintType.CHARACTER_SET to "ISO-8859-1"))
    val matrix = code.matrix
    return List(matrix.height) { y -> BooleanArray(matrix.width) { x -> matrix.get(x, y).toInt() == 1 } }
}

/**
 * 輝度（Y 面）の画像から QR を読み、バイト列を返す。見つからなければ `null`。バイトモードの中身は
 * `BYTE_SEGMENTS` から取り、文字コードの解釈で壊さない。
 */
internal fun decodeQr(luminance: ByteArray, width: Int, height: Int, rowStride: Int = width): ByteArray? {
    val source = PlanarYUVLuminanceSource(luminance, rowStride, height, 0, 0, width, height, false)
    val hints = mapOf(DecodeHintType.TRY_HARDER to true, DecodeHintType.CHARACTER_SET to "ISO-8859-1")
    val reader = QRCodeReader()
    for (candidate in listOf(source, source.invert())) {
        val result = try {
            reader.decode(BinaryBitmap(HybridBinarizer(candidate)), hints)
        } catch (_: Exception) {
            reader.reset()
            continue
        }
        @Suppress("UNCHECKED_CAST")
        val segments = result.resultMetadata?.get(ResultMetadataType.BYTE_SEGMENTS) as? List<ByteArray>
        return segments?.takeIf { it.isNotEmpty() }?.reduce { a, b -> a + b }
            ?: result.text?.toByteArray(Charsets.ISO_8859_1)
    }
    return null
}
