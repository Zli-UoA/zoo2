package zoo

import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import org.jetbrains.skia.Bitmap
import org.jetbrains.skia.Codec
import org.jetbrains.skia.Data
import org.jetbrains.skia.Image

/** Decode on demand: retain the current raster, rather than every frame of a large GIF/WebP. */
internal fun webImageFrames(bytes: ByteArray): Flow<ImageBitmap> = flow {
    val data = Data.makeFromBytes(bytes)
    val codec =
        try {
            Codec.makeFromData(data)
        } catch (e: Throwable) {
            data.close()
            throw e
        }
    val bitmap = Bitmap()
    try {
        bitmap.allocPixels(codec.imageInfo)
        val count = codec.frameCount
        var repeats = 0
        do {
            for (index in 0 until count) {
                currentCoroutineContext().ensureActive()
                val info = if (count > 1) codec.getFrameInfo(index) else null
                val prior = if (index > 0 && info?.requiredFrame == index - 1) index - 1 else -1
                codec.readPixels(bitmap, index, prior)
                emit(Image.makeFromBitmap(bitmap).toComposeImageBitmap())
                if (info != null) delay(if (info.duration <= 10) 100L else info.duration.toLong())
            }
        } while (count > 1 && (codec.repetitionCount == -1 || repeats++ < codec.repetitionCount))
    } finally {
        bitmap.close()
        codec.close()
        data.close()
    }
}
