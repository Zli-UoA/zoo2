package zoo

import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay

/** The author column and grouped-message gutter must have the same width. */
@Composable
internal fun postAvatarSize() = if (LocalTouchUi.current) 48.dp else 32.dp

internal fun Modifier.postAvatar() = semantics { testTag = "post-avatar" }

/** Refresh visible relative dates without fetching the post again. */
@Composable
internal fun PostTimestamp(app: Session, createdAt: String, relative: Boolean = true) {
    val visible = app.env.visible
    val locale = app.env.locale
    val label by produceState(app.platform.relativeTimestamp(createdAt), createdAt, relative, visible, locale) {
        while (true) {
            value = if (relative) app.platform.relativeTimestamp(createdAt) else app.platform.time(createdAt)
            if (!relative || !visible) break
            delay(30_000)
        }
    }
    val full = app.platform.fullTimestamp(createdAt)
    Hint(full) { hint ->
        Text(label, hint.then(webControlModifier("time", attributes = mapOf("datetime" to createdAt)))
            .semantics { contentDescription = full },
            size = if (LocalTouchUi.current) 14.sp else 12.sp,
            small = true, muted = true)
    }
}
