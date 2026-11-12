package zoo

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import kotlin.math.*

fun imageAspectRatio(row: Row, fallback: Float = 4f / 3f): Float {
    val width = row.text("width").toIntOrNull() ?: return fallback
    val height = row.text("height").toIntOrNull() ?: return fallback
    return if (width > 0 && height > 0) (width.toFloat() / height).coerceIn(0.1f, 10f) else fallback
}

/** Decode a small preview independently of untrusted image dimensions. */
fun decodeBlurhash(hash: String, size: Int = 24): IntArray? {
    val alphabet = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#\u0024%*+,-.:;=?@[]^_{|}~"
    if (hash.length !in 6..166 || size !in 1..32 || hash.any { it !in alphabet }) return null
    fun base83(start: Int, end: Int): Int {
        var value = 0
        for (i in start until end) value = value * 83 + alphabet.indexOf(hash[i])
        return value
    }
    val flag = base83(0, 1)
    if (flag > 80) return null
    val nx = flag % 9 + 1
    val ny = flag / 9 + 1
    if (hash.length != 4 + 2 * nx * ny) return null
    val maximum = (base83(1, 2) + 1) / 166.0
    fun linear(value: Int): Double {
        val v = value / 255.0
        return if (v <= 0.04045) v / 12.92 else ((v + 0.055) / 1.055).pow(2.4)
    }
    val coefficients = List(nx * ny) { index ->
        if (index == 0) {
            val dc = base83(2, 6)
            doubleArrayOf(linear((dc shr 16) and 255), linear((dc shr 8) and 255), linear(dc and 255))
        } else {
            val ac = base83(4 + index * 2, 6 + index * 2)
            fun component(value: Int): Double {
                val v = (value - 9) / 9.0
                return v * abs(v) * maximum
            }
            doubleArrayOf(component(ac / (19 * 19)), component((ac / 19) % 19), component(ac % 19))
        }
    }
    fun srgb(value: Double): Int {
        val v = value.coerceIn(0.0, 1.0)
        return ((if (v <= 0.0031308) v * 12.92 else 1.055 * v.pow(1 / 2.4) - 0.055) * 255 + 0.5).toInt()
    }
    val cosX = List(size) { x -> DoubleArray(nx) { cos(PI * x * it / size) } }
    val cosY = List(size) { y -> DoubleArray(ny) { cos(PI * y * it / size) } }
    return IntArray(size * size) { pixel ->
        var r = 0.0; var g = 0.0; var b = 0.0
        for (j in 0 until ny) for (i in 0 until nx) {
            val basis = cosX[pixel % size][i] * cosY[pixel / size][j]
            val c = coefficients[j * nx + i]
            r += c[0] * basis; g += c[1] * basis; b += c[2] * basis
        }
        (255 shl 24) or (srgb(r) shl 16) or (srgb(g) shl 8) or srgb(b)
    }
}

@Composable
fun BlurhashPlaceholder(hash: String, modifier: Modifier = Modifier) {
    val pixels = remember(hash) { decodeBlurhash(hash) } ?: return
    Canvas(modifier) {
        val w = size.width / 24
        val h = size.height / 24
        pixels.forEachIndexed { index, color ->
            drawRect(Color(color), Offset(index % 24 * w, index / 24 * h), Size(w + 0.5f, h + 0.5f))
        }
    }
}

/** Only local BlurHash pixels are drawn until the user requests the original image. */
@Composable
fun SensitiveImagePlaceholder(app: Session, file: Row, modifier: Modifier, reveal: () -> Unit) {
    val label = app.t("attachments.showSensitive")
    Box(modifier.clip(RoundedCornerShape(8.dp))
        .background(MaterialTheme.colorScheme.surfaceVariant)
        .semantics { contentDescription = label }
        .clickable(role = Role.Button, onClickLabel = label, onClick = reveal),
        contentAlignment = Alignment.Center) {
        BlurhashPlaceholder(file.text("blurhash"), Modifier.fillMaxSize())
        Text(label, Modifier.background(MaterialTheme.colorScheme.surface.copy(alpha = .85f), RoundedCornerShape(4.dp)).padding(8.dp), small = true)
    }
}
