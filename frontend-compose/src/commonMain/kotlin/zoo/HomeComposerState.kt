package zoo

import androidx.compose.runtime.*
import kotlinx.coroutines.*

/** One draft survives closing the composer and switching between a post and a reply. */
@Stable
class HomeComposerState(internal val settingsKey: String? = null) {
    internal var focusOnOpen: (() -> Unit)? = null
    internal var preferencesReady by mutableStateOf(settingsKey == null)

    var opened by mutableStateOf(false)
        internal set

    var content by mutableStateOf("")
        internal set

    var visibility by mutableStateOf("unlisted")
        internal set

    var federationPreference by mutableStateOf<Boolean?>(null)
        internal set

    var channelId by mutableStateOf("")
        internal set

    var replyTarget by mutableStateOf<Row?>(null)
        internal set

    var sending by mutableStateOf(false)
        internal set

    var failed by mutableStateOf(false)
        internal set

    fun open() {
        if (!sending && preferencesReady) {
            opened = true
            focusOnOpen?.invoke()
        }
    }

    fun close() {
        if (!sending) opened = false
    }

    fun reply(post: Row) {
        if (sending || !preferencesReady) return
        replyTarget = post
        channelId = ""
        visibility = homeReplyVisibility(visibility, post.text("visibility"), post.text("channel_id").isNotBlank())
        opened = true
        focusOnOpen?.invoke()
    }

    internal fun submit(scope: CoroutineScope, images: AttachmentDrafts, post: suspend () -> Unit) {
        if (sending || !scope.isActive) return
        val release = images.retainForSubmission()
        sending = true
        failed = false
        scope.launch {
            try {
                post()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                failed = true
            }
        }.invokeOnCompletion {
            release()
            sending = false
        }
    }
}
