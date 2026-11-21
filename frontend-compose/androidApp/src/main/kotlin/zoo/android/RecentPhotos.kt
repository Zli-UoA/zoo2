package zoo.android

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.MediaStore
import android.util.Base64
import android.util.Size
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import zoo.obj

/** Recent photos stay local. Only an explicit attachment becomes an upload candidate. */
internal class RecentPhotos(private val context: Context) {
    private val resolver get() = context.contentResolver
    private val collection get() = MediaStore.Images.Media.EXTERNAL_CONTENT_URI

    fun permissions(): Array<String> = when {
        Build.VERSION.SDK_INT >= 34 -> arrayOf(
            Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED,
        )
        Build.VERSION.SDK_INT >= 33 -> arrayOf(Manifest.permission.READ_MEDIA_IMAGES)
        Build.VERSION.SDK_INT >= 29 -> arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        else -> emptyArray()
    }

    private fun canRead() = permissions().any { context.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }

    suspend fun list(): String = withContext(Dispatchers.IO) {
        val supported = Build.VERSION.SDK_INT >= 29
        val allowed = supported && canRead()
        val rows = if (Build.VERSION.SDK_INT >= 29 && allowed) {
            val args = Bundle().apply {
                putStringArray(android.content.ContentResolver.QUERY_ARG_SORT_COLUMNS,
                    arrayOf(MediaStore.Images.Media.DATE_ADDED, MediaStore.Images.Media._ID))
                putInt(android.content.ContentResolver.QUERY_ARG_SORT_DIRECTION,
                    android.content.ContentResolver.QUERY_SORT_DIRECTION_DESCENDING)
                putInt(android.content.ContentResolver.QUERY_ARG_LIMIT, 8)
            }
            resolver.query(collection, arrayOf(MediaStore.Images.Media._ID, MediaStore.Images.Media.DISPLAY_NAME), args, null)
                ?.use { cursor ->
                    buildList {
                        while (size < 8 && cursor.moveToNext()) {
                            val id = cursor.getLong(0)
                            val bitmap = try { resolver.loadThumbnail(uri(id), Size(112, 112), null) }
                                catch (_: java.io.IOException) { continue }
                            val bytes = ByteArrayOutputStream().use { output ->
                                try {
                                    check(bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 80, output))
                                    output.toByteArray()
                                } finally { bitmap.recycle() }
                            }
                            add(obj("id" to id.toString(), "name" to cursor.getString(1).orEmpty(),
                                "thumbnail" to Base64.encodeToString(bytes, Base64.NO_WRAP)))
                        }
                    }
                }.orEmpty()
        } else emptyList()
        buildJsonObject { put("supported", supported); put("allowed", allowed); put("photos", JsonArray(rows)) }.toString()
    }

    fun select(id: Long): Uri {
        check(canRead())
        return uri(id)
    }

    private fun uri(id: Long): Uri = ContentUris.withAppendedId(collection, id)
}
