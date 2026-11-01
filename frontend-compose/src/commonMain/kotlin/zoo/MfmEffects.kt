package zoo

import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.*
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.*
import kotlin.math.*

val LocalMfmStyle = staticCompositionLocalOf { TextStyle() }
private val LocalMfmScale = staticCompositionLocalOf { 1f }

@Composable
fun MfmEffect(source: String, users: List<Row>, depth: Int) {
    val app = LocalApp.current
    val header = source.substringBefore(' ')
    val name = header.substringBefore('.')
    val known =
        setOf(
            "tada",
            "jelly",
            "twitch",
            "shake",
            "spin",
            "jump",
            "bounce",
            "rainbow",
            "sparkle",
            "flip",
            "scale",
            "position",
            "rotate",
            "blur",
            "border",
            "bg",
            "fg",
            "clickable",
            "ruby",
            "unixtime",
            "x2",
            "x3",
            "x4",
            "font",
        )
    if (name !in known) {
        Text("$[$source]")
        return
    }
    val inherited = LocalMfmStyle.current
    val cumulativeScale = LocalMfmScale.current
    val remainingScale = (8f / cumulativeScale).coerceAtLeast(1f)
    val args =
        header
            .substringAfter('.', "")
            .split(',')
            .filter { it.isNotEmpty() }
            .associate { it.substringBefore('=') to it.substringAfter('=', "true") }
    val text = source.substringAfter(' ', "")
    fun number(key: String, fallback: Float, min: Float = -100f, max: Float = 100f): Float =
        args[key]?.toFloatOrNull()?.takeIf { it.isFinite() }?.coerceIn(min, max) ?: fallback
    fun color(key: String, fallback: Color): Color {
        val s = args[key] ?: return fallback
        val v = s.toLongOrNull(16) ?: return fallback
        return when (s.length) {
            3 ->
                Color(
                    0xff000000 or
                        (((v shr 8) and 15) * 17 shl 16) or
                        (((v shr 4) and 15) * 17 shl 8) or
                        ((v and 15) * 17)
                )
            4 ->
                Color(
                    (((v and 15) * 17) shl 24) or
                        (((v shr 12) and 15) * 17 shl 16) or
                        (((v shr 8) and 15) * 17 shl 8) or
                        (((v shr 4) and 15) * 17)
                )
            6 -> Color(0xff000000 or v)
            8 -> Color(((v and 255) shl 24) or (v shr 8))
            else -> fallback
        }
    }
    val animated =
        name in
            setOf(
                "tada",
                "jelly",
                "twitch",
                "shake",
                "spin",
                "jump",
                "bounce",
                "rainbow",
                "sparkle",
            ) && !app.env.reducedMotion
    val fallback =
        when (name) {
            "spin",
            "sparkle" -> 1.5f
            "twitch",
            "shake" -> .5f
            "jump",
            "bounce" -> .75f
            else -> 1f
        }
    val speed =
        args["speed"]
            ?.let {
                it.removeSuffix("ms")
                    .removeSuffix("s")
                    .toFloatOrNull()
                    ?.div(if (it.endsWith("ms")) 1000 else 1)
            }
            ?.coerceIn(.5f, 86400f) ?: fallback
    val delaySeconds =
        args["delay"]
            ?.let {
                it.removeSuffix("ms")
                    .removeSuffix("s")
                    .toFloatOrNull()
                    ?.takeIf(Float::isFinite)
                    ?.div(if (it.endsWith("ms")) 1000 else 1)
            }
            ?.coerceIn(-86400f, 86400f) ?: 0f
    val phase =
        if (animated)
            rememberInfiniteTransition()
                .animateFloat(
                    0f,
                    1f,
                    infiniteRepeatable(
                        tween((speed * 1000).toInt(), easing = LinearEasing),
                        if (args.containsKey("alternate")) RepeatMode.Reverse
                        else RepeatMode.Restart,
                        initialStartOffset =
                            StartOffset(
                                (abs(delaySeconds) * 1000).toInt(),
                                if (delaySeconds < 0) StartOffsetType.FastForward
                                else StartOffsetType.Delay,
                            ),
                    ),
                )
                .value
        else 0f
    val wave = sin(phase * 2 * PI).toFloat()
    var modifier: Modifier = Modifier
    modifier =
        when (name) {
            "spin" ->
                modifier.graphicsLayer {
                    val rotation = phase * 360 * if (args.containsKey("left")) -1 else 1
                    if (args.containsKey("x")) rotationX = rotation
                    else if (args.containsKey("y")) rotationY = rotation else rotationZ = rotation
                }
            "shake",
            "twitch" ->
                modifier.offset(
                    (wave * if (name == "shake") 3 else 1).dp,
                    (cos(phase * 14 * PI) * 2).dp,
                )
            "jump",
            "bounce" -> modifier.offset(y = (-abs(wave) * 10).dp)
            "tada" ->
                modifier.graphicsLayer {
                    rotationZ = wave * 8
                    scaleX = 1 + abs(wave) * .1f
                    scaleY = scaleX
                }
            "jelly" ->
                modifier.graphicsLayer {
                    scaleX = 1 + wave * .15f
                    scaleY = 1 - wave * .15f
                }
            "flip" ->
                modifier.graphicsLayer {
                    scaleX = if (args.containsKey("h") || !args.containsKey("v")) -1f else 1f
                    scaleY = if (args.containsKey("v")) -1f else 1f
                }
            "scale" ->
                modifier.graphicsLayer {
                    scaleX = number("x", 1f, -5f, 5f).coerceIn(-remainingScale, remainingScale)
                    scaleY = number("y", 1f, -5f, 5f).coerceIn(-remainingScale, remainingScale)
                }
            "position" ->
                modifier.offset(
                    (number("x", 0f, -2f, 2f) * 14).dp,
                    (number("y", 0f, -2f, 2f) * 14).dp,
                )
            "rotate" -> modifier.rotate(number("deg", 90f, -3600f, 3600f))
            "blur" -> modifier.blur(6.dp)
            "border" -> {
                val radius = number("radius", 0f, 0f, 100f).dp
                val borderColor =
                    color(
                        "color",
                        androidx.compose.material3.MaterialTheme.colorScheme.onBackground,
                    )
                val strokeWidth = number("width", 1f, 0f, 100f).dp
                val style = args["style"]
                val drawing = modifier.drawBehind {
                    val w = strokeWidth.toPx()
                    val pattern =
                        when (style) {
                            "dashed" -> floatArrayOf(w * 3, w * 2)
                            "dotted" -> floatArrayOf(w, w)
                            else -> null
                        }
                    if (w > 0)
                        drawRoundRect(
                            borderColor,
                            cornerRadius = CornerRadius(radius.toPx()),
                            style =
                                Stroke(
                                    w,
                                    pathEffect = pattern?.let { PathEffect.dashPathEffect(it) },
                                ),
                        )
                }
                if (args.containsKey("noclip")) drawing
                else drawing.clip(RoundedCornerShape(radius))
            }
            "bg" -> modifier.background(color("color", Color.Transparent))
            "clickable" -> modifier.clickable { app.platform.action("mfm:${args["ev"].orEmpty()}") }
            else -> modifier
        }
    val scale =
        when (name) {
            "scale" ->
                max(abs(number("x", 1f, -5f, 5f)), abs(number("y", 1f, -5f, 5f)))
                    .coerceAtMost(remainingScale)
            "x2",
            "x3",
            "x4" -> name.drop(1).toFloat().coerceAtMost(remainingScale)
            else -> 1f
        }
    val style =
        when (name) {
            "x2",
            "x3",
            "x4" ->
                inherited.copy(
                    fontSize =
                        ((if (inherited.fontSize.isSp) inherited.fontSize.value else 14f) * scale)
                            .sp,
                    lineHeight =
                        ((if (inherited.lineHeight.isSp) inherited.lineHeight.value else 20f) *
                                scale)
                            .sp,
                )
            "fg" -> inherited.copy(color = color("color", inherited.color))
            "rainbow" -> inherited.copy(color = Color.hsv(phase * 360, .8f, .85f))
            else -> inherited
        }
    CompositionLocalProvider(
        LocalMfmStyle provides style,
        LocalMfmScale provides cumulativeScale * scale,
    ) {
        Box(modifier) {
            when (name) {
                "ruby" -> {
                    val parts = text.split(' ')
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(parts.drop(1).joinToString(" "), small = true)
                        RichText(parts.firstOrNull().orEmpty(), users, depth)
                    }
                }
                "unixtime" -> {
                    val time =
                        text.toLongOrNull()?.let {
                            runCatching { kotlinx.datetime.Instant.fromEpochSeconds(it).toString() }
                                .getOrNull()
                        }
                    Text(
                        time?.let { app.platform.day(it) + " " + app.platform.time(it) }
                            ?: "$[$source]"
                    )
                }
                "x2",
                "x3",
                "x4",
                "fg",
                "rainbow" -> RichText(text, users, depth)
                "sparkle" ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("✦", color = Color(0xffffbb00).copy(alpha = .5f + abs(wave) * .5f))
                        RichText(text, users, depth)
                        Text("✧", color = Color(0xffffbb00))
                    }
                "font" -> {
                    val family =
                        when {
                            args.containsKey("monospace") -> LocalFonts.current.mono
                            args.containsKey("serif") || args.containsKey("math") ->
                                FontFamily.Serif
                            args.containsKey("cursive") || args.containsKey("fantasy") ->
                                FontFamily.Cursive
                            args.containsKey("emoji") -> LocalFonts.current.emoji
                            else -> LocalFonts.current.cjk
                        }
                    CompositionLocalProvider(
                        LocalFonts provides LocalFonts.current.copy(latin = family, cjk = family)
                    ) {
                        RichText(text, users, depth)
                    }
                }
                else -> RichText(text, users, depth)
            }
        }
    }
}
