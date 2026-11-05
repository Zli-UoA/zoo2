package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** 36px rail logo + 12px above and below. Callers add the shared 1px divider. */
@Composable
fun PaneHeader(
    modifier: Modifier = Modifier,
    horizontalPadding: Dp = 16.dp,
    content: @Composable RowScope.() -> Unit,
) {
    val platform = LocalWebPlatform.current
    val tag = remember { "pane-header:${platform?.uuid().orEmpty()}" }
    Row(
        modifier.fillMaxWidth().height(60.dp).padding(horizontal = horizontalPadding).semantics {
            testTag = tag
        },
        verticalAlignment = Alignment.CenterVertically,
        content = content,
    )
}

@Composable
fun ScreenHeader(
    icon: String,
    title: String,
    description: String? = null,
    primaryIcon: Boolean = false,
    actions: @Composable RowScope.() -> Unit = {},
) {
    PaneHeader {
        Box(
            Modifier.size(32.dp)
                .background(
                    if (primaryIcon) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.secondaryContainer,
                    RoundedCornerShape(10.dp),
                ),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                icon,
                modifier = Modifier.size(16.dp),
                color =
                    if (primaryIcon) MaterialTheme.colorScheme.onPrimary
                    else MaterialTheme.colorScheme.onSecondaryContainer,
            )
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                title,
                Modifier.then(
                        webControlModifier("heading", attributes = mapOf("aria-level" to "1"))
                    )
                    .semantics { heading() },
                weight = FontWeight.SemiBold,
                maxLines = 1,
            )
            description?.let { Text(it, small = true, muted = true, maxLines = 1) }
        }
        actions()
    }
}
