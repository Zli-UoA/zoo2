package zoo.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import androidx.exifinterface.media.ExifInterface
import java.io.ByteArrayInputStream

/** Match browser EXIF handling on every supported Android version, including API 26. */
internal fun decodeOrientedBitmap(bytes: ByteArray, maxEdge: Int = 2048): Bitmap {
    val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
    options.inSampleSize = imageSampleSize(options.outWidth, options.outHeight, maxEdge)
    options.inJustDecodeBounds = false
    val bitmap = requireNotNull(BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)) { "Invalid image" }
    val orientation = runCatching {
        ExifInterface(ByteArrayInputStream(bytes))
            .getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)
    }.getOrDefault(ExifInterface.ORIENTATION_NORMAL)
    val matrix = Matrix()
    when (orientation) {
        ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> matrix.setScale(-1f, 1f)
        ExifInterface.ORIENTATION_ROTATE_180 -> matrix.setRotate(180f)
        ExifInterface.ORIENTATION_FLIP_VERTICAL -> {
            matrix.setRotate(180f)
            matrix.postScale(-1f, 1f)
        }
        ExifInterface.ORIENTATION_TRANSPOSE -> {
            matrix.setRotate(90f)
            matrix.postScale(-1f, 1f)
        }
        ExifInterface.ORIENTATION_ROTATE_90 -> matrix.setRotate(90f)
        ExifInterface.ORIENTATION_TRANSVERSE -> {
            matrix.setRotate(-90f)
            matrix.postScale(-1f, 1f)
        }
        ExifInterface.ORIENTATION_ROTATE_270 -> matrix.setRotate(-90f)
        else -> return bitmap
    }
    val oriented = Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, matrix, true)
    if (oriented !== bitmap) bitmap.recycle()
    return oriented
}

/** Bound decoded previews before allocating a bitmap (2048 pixels / 16 MiB by default). */
internal fun imageSampleSize(width: Int, height: Int, maxEdge: Int = 2048): Int {
    require(width > 0 && height > 0) { "Invalid image dimensions" }
    require(maxEdge > 0) { "Invalid image limit" }
    var sample = 1
    while ((maxOf(width, height).toLong() + sample - 1) / sample > maxEdge) sample *= 2
    return sample
}
