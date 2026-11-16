package zoo

import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.*
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.*
import kotlin.math.*

val LocalMfmStyle = staticCompositionLocalOf { TextStyle() }
internal val LocalMfmInLink = staticCompositionLocalOf { false }
private val LocalMfmScale = staticCompositionLocalOf { 1f }

@Composable
fun MfmEffect(source: String, users: List<Row>, depth: Int) {
    val app = LocalApp.current
    val function = remember(source) { mfmFunction(source) }
    val name = function.name
    val args = function.args
    val text = function.body
    if (name !in supportedMfmFunctions) {
        Text("$[$source]")
        return
    }
    val inherited = androidx.compose.material3.MaterialTheme.typography.bodyMedium
        .merge(LocalMfmStyle.current)
    val cumulativeScale = LocalMfmScale.current
    val sizing = function.scale(cumulativeScale)
    fun number(key: String, fallback: Float, min: Float = -100f, max: Float = 100f) =
        function.number(key, fallback, min, max)
    fun color(key: String, fallback: Color) = function.argb(key)?.let { Color(it) } ?: fallback
    val animated = name in mfmAnimationDurations && !app.env.reducedMotion
    val speed = function.seconds("speed", mfmAnimationDurations[name] ?: 1f)
    val delaySeconds = function.seconds("delay", 0f, delay = true)
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
    val blurInteraction = remember { MutableInteractionSource() }
    val hovered by blurInteraction.collectIsHoveredAsState()
    var focused by remember { mutableStateOf(false) }
    var revealed by remember(source) { mutableStateOf(false) }
    val motion = function.motion(phase, app.env.reducedMotion, cumulativeScale)
    val em = with(androidx.compose.ui.platform.LocalDensity.current) { inherited.fontSize.toPx() }
    var modifier: Modifier = Modifier.graphicsLayer {
        scaleX = motion.scaleX
        scaleY = motion.scaleY
        rotationX = motion.rotationX
        rotationY = motion.rotationY
        rotationZ = motion.rotationZ
        translationX = motion.x * em
        translationY = motion.y * em
    }
    modifier = when (name) {
            "blur" -> modifier
                .hoverable(blurInteraction)
                .onFocusChanged { focused = it.isFocused }
                .semantics { selected = revealed || hovered || focused }
                .clickable(role = Role.Button) { revealed = !revealed }
                .then(if (revealed || hovered || focused) Modifier else Modifier.blur(6.dp))
            "border" -> {
                val radius = number("radius", 0f, 0f, 1000f).dp
                val borderColor =
                    color(
                        "color",
                        androidx.compose.material3.MaterialTheme.colorScheme.onBackground,
                    )
                val strokeWidth = number("width", 1f, 0f, 100f).dp
                val style = args["style"]
                val drawing = modifier.drawBehind {
                    val w = minOf(strokeWidth.toPx(), size.minDimension / 2)
                    val r = radius.toPx().coerceAtMost(size.minDimension / 2)
                    fun stroke(inset: Float, width: Float, first: Color, second: Color = first) {
                        val extent = Size((size.width - inset * 2).coerceAtLeast(0f),
                            (size.height - inset * 2).coerceAtLeast(0f))
                        val pattern = when (style) {
                            "dashed" -> floatArrayOf(width * 3, width * 2)
                            "dotted" -> floatArrayOf(width, width)
                            else -> null
                        }
                        val line = Stroke(width, pathEffect = pattern?.let { PathEffect.dashPathEffect(it) })
                        fun draw(color: Color) = drawRoundRect(color, Offset(inset, inset), extent,
                            CornerRadius((r - inset).coerceAtLeast(0f)), style = line)
                        draw(second)
                        if (first != second) {
                            val upperLeft = Path().apply {
                                moveTo(0f, size.height)
                                lineTo(0f, 0f)
                                lineTo(size.width, 0f)
                                close()
                            }
                            clipPath(upperLeft) { draw(first) }
                        }
                    }
                    if (w > 0 && style !in setOf("hidden", "none")) {
                        val dark = lerp(borderColor, Color.Black, .4f)
                        val light = lerp(borderColor, Color.White, .4f)
                        when (style) {
                            "double" -> {
                                stroke(w / 6, w / 3, borderColor)
                                stroke(w * 5 / 6, w / 3, borderColor)
                            }
                            "inset" -> stroke(w / 2, w, dark, light)
                            "outset" -> stroke(w / 2, w, light, dark)
                            "groove", "ridge" -> {
                                val first = if (style == "groove") dark else light
                                val second = if (style == "groove") light else dark
                                stroke(w / 4, w / 2, first, second)
                                stroke(w * 3 / 4, w / 2, second, first)
                            }
                            else -> stroke(w / 2, w, borderColor)
                        }
                    }
                }.padding(if (style in setOf("hidden", "none")) 0.dp else strokeWidth)
                if (args.containsKey("noclip")) drawing
                else drawing.clip(RoundedCornerShape(radius))
            }
            "bg" -> modifier.background(color("color", Color.Red))
            "clickable" -> if (LocalMfmInLink.current) modifier else modifier.clickable(role = Role.Button) { app.platform.action("mfm:${args["ev"].orEmpty()}") }
            else -> modifier
        }
    val style =
        when (name) {
            "x2",
            "x3",
            "x4" ->
                inherited.copy(
                    fontSize =
                        ((if (inherited.fontSize.isSp) inherited.fontSize.value else 14f) * sizing.x)
                            .sp,
                    lineHeight =
                        ((if (inherited.lineHeight.isSp) inherited.lineHeight.value else 20f) *
                                sizing.x)
                            .sp,
                )
            "fg" -> inherited.copy(color = color("color", Color.Red))
            "rainbow" -> inherited.copy(color = Color.hsv(phase * 360, .8f, .85f))
            else -> inherited
        }
    CompositionLocalProvider(
        LocalMfmStyle provides style,
        LocalMfmScale provides sizing.cumulative,
    ) {
        Box(modifier) {
            when (name) {
                "ruby" -> {
                    val ruby = remember(text) { mfmRuby(text) }
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            ruby.reading,
                            size = (inherited.fontSize.value * .5f).sp,
                            lineHeight = (inherited.fontSize.value * .65f).sp,
                        )
                        RichText(ruby.base, users, depth, selection = false)
                    }
                }
                "unixtime" -> {
                    val time =
                        mfmUnixSeconds(text)?.let {
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
                    Box {
                        Text("✦", Modifier.align(Alignment.TopStart), color = Color(0xffffbb00).copy(alpha = .5f + abs(wave) * .5f))
                        RichText(text, users, depth)
                        Text("✧", Modifier.align(Alignment.BottomEnd), color = Color(0xffffbb00))
                    }
                "font" -> {
                    val fonts = LocalFonts.current
                    val family = when (mfmFont(args)) {
                        "serif" -> FontFamily.Serif
                        "monospace" -> fonts.mono
                        "cursive", "fantasy" -> FontFamily.Cursive
                        "emoji" -> fonts.emoji
                        "math" -> fonts.math
                        else -> null
                    }
                    CompositionLocalProvider(
                        LocalFonts provides if (family == null) fonts
                            else fonts.copy(latin = family, cjk = family)
                    ) {
                        RichText(text, users, depth)
                    }
                }
                else -> RichText(text, users, depth)
            }
        }
    }
}
