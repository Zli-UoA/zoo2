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
import androidx.compose.foundation.clickable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.layout.*
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
import androidx.compose.ui.unit.Dp
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
        CallPortrait(call, colors, 88.dp)
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
        Text(label, color = colors.foreground, fontSize = 12.sp, fontWeight = AndroidWeights.emphasis)
    }
}

@Composable
internal fun IncomingCallAction(label: String, icon: UiIcon, fill: Color, colors: AndroidColors, action: () -> Unit) {
    Column(
        Modifier.clickable(role = Role.Button, onClick = action),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(Modifier.size(76.dp).testTag("incoming-call-$label").background(fill, CircleShape),
            contentAlignment = Alignment.Center) {
            UiGlyph(icon, Color.White, iconSize = 36.dp)
        }
        Text(label, color = colors.foreground, fontSize = 13.sp, fontWeight = AndroidWeights.emphasis)
    }
}


@Composable
internal fun CallPortrait(call: AndroidScreen.Call, colors: AndroidColors, size: Dp) {
    val modifier = Modifier.size(size).clip(CircleShape).testTag("call-avatar")
    val avatar = call.avatar
    when {
        avatar != null -> Image(avatar, contentDescription = null, modifier, contentScale = ContentScale.Crop)
        call.hasUser -> Box(modifier.background(colors.headerLight), contentAlignment = Alignment.Center) {
            Text(call.caller.take(1), color = colors.foreground, fontSize = (size.value * .36f).sp,
                fontWeight = AndroidWeights.userName)
        }
        else -> ZooBadge(colors, modifier)
    }
}

@Composable
internal fun CallPage(
    call: AndroidScreen.Call, colors: AndroidColors, motion: Boolean,
    remoteAudioLevel: () -> Float, dispatch: (AndroidUiAction) -> Unit,
    answerLabel: String = "応答", dismissLabel: String = "拒否",
) {
    BrandPage(call.status ?: if (call.incoming) "着信中" else "通話中", call.caller, colors,
        detailWeight = AndroidWeights.userName, badge = { CallAvatar(call, colors, motion, remoteAudioLevel) }) {
        if (call.incoming) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.SpaceAround,
            ) {
                IncomingCallAction(answerLabel, UiIcon.Phone, Color(0xff1b743d), colors) { dispatch(AndroidUiAction.Answer) }
                IncomingCallAction(dismissLabel, UiIcon.PhoneOff, Color(0xffbc3037), colors) { dispatch(AndroidUiAction.HangUp) }
            }
        } else Column(Modifier.width(IntrinsicSize.Max), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (call.minimize || call.pictureInPicture) Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) {
                if (call.minimize) UiIconButton(UiIcon.Minimize, "通話画面をしまう", colors.foreground,
                    { dispatch(AndroidUiAction.MinimizeCall) })
                if (call.pictureInPicture) UiIconButton(UiIcon.Float, "通話を小窓で表示", colors.foreground,
                    { dispatch(AndroidUiAction.FloatCall) })
            }
            if (call.active) Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterHorizontally)) {
                CallToggle("ミュート", if (call.muted) UiIcon.MicOff else UiIcon.Mic, call.muted, colors) {
                    dispatch(AndroidUiAction.MuteCall)
                }
                CallToggle("スピーカー", if (call.speaker) UiIcon.Speaker else UiIcon.SpeakerOff, call.speaker, colors) {
                    dispatch(AndroidUiAction.SpeakerCall)
                }
            }
            TextAction("通話を終了", colors, style = TextActionStyle.Neutral, modifier = Modifier.fillMaxWidth(),
                icon = UiIcon.PhoneOff, labelWeight = AndroidWeights.emphasis) { dispatch(AndroidUiAction.HangUp) }
        }
    }
}
