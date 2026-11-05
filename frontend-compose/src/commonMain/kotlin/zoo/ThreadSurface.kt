package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*

@Composable
internal fun ThreadSurface(
    app: Session,
    count: Int,
    loading: Boolean,
    modifier: Modifier = Modifier,
    close: () -> Unit,
    root: @Composable () -> Unit,
    replies: @Composable ColumnScope.() -> Unit,
    composer: (@Composable () -> Unit)?,
) {
    val windowWidth =
        with(LocalDensity.current) { LocalWindowInfo.current.containerSize.width.toDp() }
    val compact = windowWidth < 640.dp
    Column(
        modifier
            .then(
                webControlModifier(
                    "complementary",
                    attributes = mapOf("aria-label" to app.t("threads.title")),
                )
            )
            .then(
                if (compact) Modifier.fillMaxWidth()
                else Modifier.width(if (windowWidth >= 1280.dp) 384.dp else 320.dp)
            )
            .fillMaxHeight()
            .background(MaterialTheme.colorScheme.background)
    ) {
        PaneHeader {
            if (compact)
                IconButton("arrow-left", app.t("threads.close"), size = 44.dp, click = close)
            Column(Modifier.weight(1f)) {
                Text(app.t("threads.title"), weight = FontWeight.SemiBold)
                Text(
                    app.t(
                        if (loading) "threads.loading" else "threads.replyCount",
                        "count" to count.toString(),
                    ),
                    small = true,
                    muted = true,
                )
            }
            if (!compact) IconButton("x", app.t("threads.close"), size = 28.dp, click = close)
        }
        HorizontalDivider()
        Box(
            Modifier.fillMaxWidth()
                .heightIn(max = 240.dp)
                .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .3f))
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 8.dp, vertical = 8.dp)
        ) {
            root()
        }
        HorizontalDivider()
        Column(Modifier.weight(1f), content = replies)
        composer?.invoke()
    }
}
