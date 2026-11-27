package zoo

import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

/** Android uses native media renderers and a bundled document/mail reader. */
@Composable
internal expect fun MediaPreviewSurface(
    app: Session,
    src: String,
    name: String,
    contentType: String,
    modifier: Modifier,
    attachmentId: String? = null,
)

internal fun hasMediaPreview(contentType: String, name: String = ""): Boolean {
    val mime = contentType.substringBefore(';').trim().lowercase()
    if (mime in imageTypes || mime == "image/avif" || mime == "application/pdf") return true
    if (mime in setOf("video/mp4", "video/webm", "video/ogg", "video/quicktime", "audio/mpeg", "audio/mp4", "audio/wav", "audio/x-wav", "audio/ogg", "audio/webm", "audio/flac", "audio/aac")) return true
    if (mime.startsWith("text/") || mime.startsWith("application/vnd.openxmlformats-officedocument.") || mime.startsWith("application/vnd.oasis.opendocument.")) return true
    if (mime in setOf("application/javascript", "application/json", "application/xml", "application/yaml", "application/toml", "message/rfc822", "application/vnd.ms-outlook", "application/zip", "application/x-zip-compressed")) return true
    return mime in setOf("", "application/octet-stream", "binary/octet-stream") && Regex("\\.(eml|msg|docx|xlsx|pptx|odt|ods|odp|zip|txt|md|csv|json|xml|html?|log|ya?ml|toml|ini|ics|vcf|sql|js|ts|py|rs|kt|css)$", RegexOption.IGNORE_CASE).containsMatchIn(name)
}

/** Native preview window: one bounded surface, without a second web modal. */
@Composable
internal expect fun MediaPreviewPanel(title: String, close: () -> Unit, content: @Composable ColumnScope.() -> Unit)
