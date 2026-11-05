package zoo

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.dp

/** Uses the same supplied geometry and brand colors as the React logo asset. */
@Composable
internal fun BrandLogo(modifier: Modifier = Modifier) {
    val paths = remember { zooBrandPaths.map { PathParser().parsePathString(it).toPath() } }
    Canvas(modifier.size(36.dp)) {
        drawRect(Color(0xffffd600))
        withTransform({
            scale(size.width / 256, size.height / 256, Offset.Zero)
            translate(42.905f, 41.975f)
            scale(.93f, .93f, Offset.Zero)
        }) {
            paths.forEach { drawPath(it, Color.Black) }
        }
    }
}
