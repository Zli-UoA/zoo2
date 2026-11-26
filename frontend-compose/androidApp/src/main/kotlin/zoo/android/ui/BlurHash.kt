package zoo.android.ui

import android.graphics.Bitmap
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.roundToInt
import kotlin.math.sign

private const val base83 = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~"
private const val previewSize = 24

/** Decode only the small placeholder, regardless of the original image dimensions. */
internal fun decodeBlurHash(hash: String): ImageBitmap? {
    if (hash.length !in 6..166) return null
    val digits = hash.map { base83.indexOf(it) }
    if (digits.any { it < 0 }) return null
    val size = digits[0]
    if (size >= 81) return null
    val columns = size % 9 + 1
    val rows = size / 9 + 1
    val count = columns * rows
    if (hash.length != 4 + count * 2) return null
    val dc = digits.subList(2, 6).fold(0) { value, digit -> value * 83 + digit }
    if (dc > 0xffffff) return null

    val scale = (digits[1] + 1) / 166.0
    val red = DoubleArray(count)
    val green = DoubleArray(count)
    val blue = DoubleArray(count)
    red[0] = toLinear(dc shr 16)
    green[0] = toLinear(dc shr 8 and 255)
    blue[0] = toLinear(dc and 255)
    for (index in 1 until count) {
        val value = digits[4 + index * 2] * 83 + digits[5 + index * 2]
        if (value >= 19 * 19 * 19) return null
        red[index] = decodeComponent(value / (19 * 19), scale)
        green[index] = decodeComponent(value / 19 % 19, scale)
        blue[index] = decodeComponent(value % 19, scale)
    }

    val basis = Array(9) { component ->
        DoubleArray(previewSize) { pixel -> cos(PI * component * pixel / previewSize) }
    }
    val pixels = IntArray(previewSize * previewSize) { pixel ->
        val x = pixel % previewSize
        val y = pixel / previewSize
        var r = 0.0
        var g = 0.0
        var b = 0.0
        for (index in 0 until count) {
            val weight = basis[index % columns][x] * basis[index / columns][y]
            r += red[index] * weight
            g += green[index] * weight
            b += blue[index] * weight
        }
        (255 shl 24) or (toSrgb(r) shl 16) or (toSrgb(g) shl 8) or toSrgb(b)
    }
    return Bitmap.createBitmap(pixels, previewSize, previewSize, Bitmap.Config.ARGB_8888).asImageBitmap()
}

private fun decodeComponent(value: Int, scale: Double): Double {
    val centered = (value - 9) / 9.0
    return sign(centered) * centered * centered * scale
}

private fun toLinear(channel: Int): Double {
    val srgb = channel / 255.0
    return if (srgb <= 0.04045) srgb / 12.92 else ((srgb + 0.055) / 1.055).pow(2.4)
}

private fun toSrgb(linear: Double): Int {
    val value = linear.coerceIn(0.0, 1.0)
    val srgb = if (value <= 0.0031308) value * 12.92 else 1.055 * value.pow(1.0 / 2.4) - 0.055
    return (srgb * 255).roundToInt()
}
