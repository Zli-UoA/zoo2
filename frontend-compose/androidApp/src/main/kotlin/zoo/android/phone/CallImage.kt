package zoo.android.phone

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.produceState
import androidx.compose.ui.graphics.ImageBitmap
import kotlinx.coroutines.CancellationException
import zoo.Session

@Composable
internal fun rememberCallImage(session: Session?, url: String?): ImageBitmap? = key(session, url) {
    val image by produceState<ImageBitmap?>(null) {
        if (session != null && !url.isNullOrBlank()) {
            value = try {
                session.imageBitmap(url)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                null
            }
        }
    }
    image
}
