package zoo.android

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Process
import androidx.core.content.IntentCompat

internal data class IncomingShare(val text: String, val images: List<Uri>)

internal fun Context.incomingShare(intent: Intent): IncomingShare? {
    val images = when (intent.action) {
        Intent.ACTION_SEND -> listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
        Intent.ACTION_SEND_MULTIPLE -> IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
        else -> return null
    }.ifEmpty {
        val clip = intent.clipData
        (0 until (clip?.itemCount ?: 0)).mapNotNull { clip?.getItemAt(it)?.uri }
    }.distinct()
    if (images.size > 10 || images.any { uri ->
        uri.scheme != "content" || uri.authority.orEmpty().substringAfterLast('@').startsWith("$packageName.") ||
            checkUriPermission(uri, Process.myPid(), Process.myUid(), Intent.FLAG_GRANT_READ_URI_PERMISSION) != PackageManager.PERMISSION_GRANTED
    }) return null
    val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString().orEmpty()
    return IncomingShare(text, images).takeIf { text.isNotBlank() || images.isNotEmpty() }
}
