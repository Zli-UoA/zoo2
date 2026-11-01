package zoo

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.*

/** Shared federated-post identity, using the same CC0 Simple Icons path as React. */
@Composable
fun ActivityPubBadge(app: Session, visibility: String, modifier: Modifier = Modifier) {
    val audience = visibility.takeIf { it in setOf("unlisted", "followers", "local") } ?: "public"
    val path = remember {
        PathParser()
            .parsePathString(
                "M10.91 4.442L0 10.74v2.52L8.727 8.22v10.077l2.182 1.26zM6.545 12l-4.364 2.52 4.364 2.518zm6.545-2.52L17.455 12l-4.364 2.52zm0-5.038L24 10.74v2.52l-10.91 6.298v-2.52L21.819 12 13.091 6.96z"
            )
            .toPath()
    }
    val color = MaterialTheme.colorScheme.onSurfaceVariant
    Hint("ActivityPub · ${app.t("home.visibility.$audience")}") { hint ->
        Row(
            modifier.then(hint),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Canvas(
                Modifier.size(14.dp).then(webControlModifier("img")).semantics {
                    role = Role.Image
                    contentDescription = "ActivityPub"
                }
            ) {
                withTransform({ scale(size.width / 24, size.height / 24, Offset.Zero) }) {
                    drawPath(path, color)
                }
            }
            Icon(
                when (audience) {
                    "public" -> "globe"
                    "followers" -> "lock-keyhole"
                    else -> "home"
                },
                app.t("home.visibility.$audience"),
                Modifier.size(14.dp).then(webControlModifier("img")),
                color,
            )
        }
    }
}
