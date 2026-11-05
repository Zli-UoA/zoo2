package zoo

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.*

/** The supplied Zoo SVG geometry, used by the application and token catalogue. */
@Composable
fun ZooMark(size: Dp, modifier: Modifier = Modifier) {
    val paths = remember {
        listOf(
            PathParser()
                .parsePathString(
                    "M91.0854 60C107.654 60 121.085 46.5685 121.085 30C121.085 13.4315 107.654 0 91.0854 0C74.5169 0 61.0854 13.4315 61.0854 30C61.0854 46.5685 74.5169 60 91.0854 60Z"
                )
                .toPath(),
            PathParser()
                .parsePathString(
                    "M152.171 82.2368C168.739 82.2368 182.171 68.8054 182.171 52.2368C182.171 35.6683 168.739 22.2368 152.171 22.2368C135.602 22.2368 122.171 35.6683 122.171 52.2368C122.171 68.8054 135.602 82.2368 152.171 82.2368Z"
                )
                .toPath(),
            PathParser()
                .parsePathString(
                    "M30 82.2368C46.5685 82.2368 60 68.8054 60 52.2368C60 35.6683 46.5685 22.2368 30 22.2368C13.4315 22.2368 0 35.6683 0 52.2368C0 68.8054 13.4315 82.2368 30 82.2368Z"
                )
                .toPath(),
            PathParser()
                .parsePathString(
                    "M91.0859 65C124.223 65.0003 151.086 91.8631 151.086 125C151.086 157.878 124.641 184.58 91.8613 184.995L91.0859 185H37.0859C33.7722 185 31.085 182.314 31.085 179V125C31.085 91.8629 57.9489 65 91.0859 65Z"
                )
                .toPath(),
        )
    }
    Canvas(modifier.size(size).clip(RoundedCornerShape(20)).background(Color(0xffffd600))) {
        scale(
            this.size.width / 256f,
            this.size.height / 256f,
            pivot = androidx.compose.ui.geometry.Offset.Zero,
        ) {
            translate(42.905f, 41.975f) {
                scale(.93f, .93f, pivot = androidx.compose.ui.geometry.Offset.Zero) {
                    paths.forEach { drawPath(it, Color.Black) }
                }
            }
        }
    }
}
