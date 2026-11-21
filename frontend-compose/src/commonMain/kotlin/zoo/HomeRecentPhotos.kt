package zoo

import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.serialization.json.jsonObject
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

/** Renders local thumbnails and uses the existing attachment owner on selection. */
@OptIn(ExperimentalEncodingApi::class)
@Composable
internal fun HomeRecentPhotos(app: Session, images: AttachmentDrafts, enabled: Boolean) {
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf<Row?>(null) }
    var loading by remember { mutableStateOf(true) }
    var failed by remember { mutableStateOf(false) }
    var selecting by remember { mutableStateOf(false) }
    suspend fun load(operation: String) {
        loading = true
        failed = false
        try {
            state = json.parseToJsonElement(app.platform.invoke(operation, "")).jsonObject
        } catch (cancel: CancellationException) {
            throw cancel
        } catch (_: Exception) {
            state = null
            failed = true
        } finally {
            loading = false
        }
    }
    LaunchedEffect(Unit) { load("recentPhotos") }
    val current = state
    androidx.compose.material3.HorizontalDivider()
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(app.t("attachments.recentPhotos"), Modifier.weight(1f), small = true)
        Button(app.t("attachments.systemPhotos"), variant = "ghost", size = "sm", enabled = enabled && !selecting) {
            images.pick(photos = true)
        }
    }
    when {
        loading -> Text(app.t("common.loading"), small = true, muted = true)
        failed -> Text(app.t("home.imageError"), small = true, muted = true)
        current?.flag("supported") != true -> Unit
        !current.flag("allowed") -> Button(app.t("attachments.allowRecentPhotos"),
            variant = "outline", size = "sm", enabled = enabled) {
            scope.launch { load("requestRecentPhotos") }
        }
        else -> Row(Modifier.horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(AndroidDesign.spaceSmall)) {
            val photos = current.array("photos").rows()
            if (photos.isEmpty()) Text(app.t("attachments.noRecentPhotos"), small = true, muted = true)
            for (photo in photos) {
                val id = photo.text("id")
                key(id, photo.text("thumbnail")) {
                    val bitmap by produceState<ImageBitmap?>(null) {
                        value = app.platform.imageFrames(Base64.decode(photo.text("thumbnail"))).first()
                    }
                    val selected = images.items.any { it.file.text("media_store_id") == id }
                    bitmap?.let {
                        Image(it, photo.text("name").ifBlank { app.t("attachments.add") },
                            Modifier.size(64.dp).clip(RoundedCornerShape(AndroidDesign.controlRadius))
                                .clickable(enabled && !selected && !selecting, role = Role.Button) {
                                    selecting = true
                                    scope.launch {
                                        try {
                                            images.add(app.platform.invoke("pickRecentPhoto", id))
                                        } catch (cancel: CancellationException) {
                                            throw cancel
                                        } catch (_: Exception) {
                                            images.error = app.t("home.imageError")
                                        } finally {
                                            selecting = false
                                        }
                                    }
                                },
                            contentScale = ContentScale.Crop,
                            alpha = if (selected) .55f else 1f,
                        )
                    }
                }
            }
        }
    }
}
