package zoo.android.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.sqrt

@Composable
internal fun CallAvatar(
    call: AndroidScreen.Call,
    colors: AndroidColors,
    motion: Boolean,
    remoteAudioLevel: () -> Float,
) {
    Box(Modifier.size(152.dp), contentAlignment = Alignment.Center) {
        if (call.incoming && motion) {
            val ring = rememberInfiniteTransition(label = "Incoming call")
            val phase = ring.animateFloat(0f, 1f, infiniteRepeatable(tween(2_400, easing = LinearEasing)),
                label = "Ring phase")
            Canvas(Modifier.matchParentSize()) {
                val radius = 45.dp.toPx()
                val cycle = (phase.value + .26f) % 1f
                repeat(2) { index ->
                    val progress = (cycle - index * .16f) / .48f
                    if (progress in 0f..1f) {
                        drawCircle(colors.accent.copy(alpha = .55f * (1f - progress)),
                            radius + 29.dp.toPx() * progress, style = Stroke(1.5.dp.toPx()))
                    }
                }
            }
        } else if (call.active && motion) {
            val level = sqrt(remoteAudioLevel().coerceIn(0f, 1f))
            val envelope = animateFloatAsState(level, tween(if (level == 0f) 260 else 110),
                label = "Remote voice")
            Canvas(Modifier.matchParentSize()) {
                val amplitude = envelope.value
                if (amplitude > .01f) {
                    drawCircle(colors.accent.copy(alpha = amplitude * .22f),
                        45.dp.toPx() + 23.dp.toPx() * amplitude)
                    drawCircle(colors.accent.copy(alpha = amplitude * .7f),
                        45.dp.toPx() + 28.dp.toPx() * amplitude, style = Stroke(1.5.dp.toPx()))
                    drawCircle(colors.accent.copy(alpha = amplitude * .35f),
                        45.dp.toPx() + 17.dp.toPx() * amplitude, style = Stroke(1.dp.toPx()))
                }
            }
        }
        val modifier = Modifier.size(88.dp).clip(CircleShape).testTag("call-avatar")
        val avatar = call.avatar
        when {
            avatar != null -> Image(avatar, contentDescription = null, modifier, contentScale = ContentScale.Crop)
            call.hasUser -> Box(modifier.background(colors.headerLight), contentAlignment = Alignment.Center) {
                Text(call.caller.take(1), color = colors.foreground, fontSize = 32.sp, fontWeight = AndroidWeights.userName)
            }
            else -> ZooBadge(colors, modifier)
        }
    }
}

@Composable
internal fun CallToggle(label: String, icon: UiIcon, active: Boolean, colors: AndroidColors, toggle: () -> Unit) {
    Column(
        Modifier.widthIn(min = 76.dp).heightIn(min = AndroidControlSize.target)
            .toggleable(active, role = Role.Switch) { toggle() }.padding(vertical = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Box(
            Modifier.size(AndroidControlSize.target).background(if (active) colors.accent else colors.inputSurface, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            UiGlyph(icon, if (active) Color.Black else colors.foreground)
        }
        Text(label, color = colors.foreground, fontSize = 12.sp, fontWeight = AndroidWeights.label)
    }
}
