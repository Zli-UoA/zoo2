package zoo

import kotlin.math.abs
import kotlin.math.max

/** Validated function settings shared by the renderer and its regression tests. */
data class MfmFunction(val name: String, val args: Map<String, String>, val body: String) {
    fun number(key: String, fallback: Float, min: Float, max: Float): Float {
        val raw = args[key] ?: return fallback
        if (!decimal.matches(raw)) return fallback
        return raw.toFloatOrNull()?.takeIf { it.isFinite() }?.coerceIn(min, max) ?: fallback
    }

    fun seconds(key: String, fallback: Float, delay: Boolean = false): Float {
        val raw = args[key] ?: return fallback
        if (!duration.matches(raw)) return fallback
        val milliseconds = raw.endsWith("ms")
        val value = raw.removeSuffix(if (milliseconds) "ms" else "s").toFloatOrNull()
            ?.takeIf { it.isFinite() } ?: return fallback
        return (value / if (milliseconds) 1000 else 1)
            .coerceIn(if (delay) -86400f else .5f, 86400f)
    }

    fun argb(key: String): Long? {
        val raw = args[key] ?: return null
        if (!hex.matches(raw)) return null
        val expanded = if (raw.length < 5) raw.map { "$it$it" }.joinToString("") else raw
        val rgb = expanded.take(6).toLong(16)
        val alpha = if (expanded.length == 8) expanded.takeLast(2).toLong(16) else 255L
        return (alpha shl 24) or rgb
    }

    fun scale(inherited: Float): MfmScale {
        val budget = (8f / inherited.coerceAtLeast(1f)).coerceIn(0f, 8f)
        if (name in setOf("x2", "x3", "x4")) {
            val factor = name.drop(1).toFloat().coerceAtMost(budget)
            return MfmScale(factor, factor, inherited * factor)
        }
        if (name != "scale") return MfmScale(1f, 1f, inherited)
        val x = number("x", 1f, -5f, 5f)
        val y = number("y", 1f, -5f, 5f)
        val magnitude = max(1f, max(abs(x), abs(y)))
        val correction = minOf(1f, budget / magnitude)
        return MfmScale(x * correction, y * correction, inherited * max(1f, magnitude * correction))
    }
}

data class MfmScale(val x: Float, val y: Float, val cumulative: Float)

val mfmAnimationDurations = mapOf(
    "tada" to 1f, "jelly" to 1f, "twitch" to .5f, "shake" to .5f,
    "spin" to 1.5f, "jump" to .75f, "bounce" to .75f, "rainbow" to 1f, "sparkle" to 1.5f,
)
val supportedMfmFunctions = mfmAnimationDurations.keys + setOf(
    "flip", "x2", "x3", "x4", "scale", "position", "fg", "bg", "border", "font",
    "blur", "rotate", "ruby", "unixtime", "clickable",
)

fun mfmFunction(source: String): MfmFunction {
    val separator = source.indexOfFirst { it.isWhitespace() }
    val header = if (separator < 0) source else source.take(separator)
    return MfmFunction(
        header.substringBefore('.'),
        header.substringAfter('.', "").split(',').filter { it.isNotEmpty() }
            .associate { it.substringBefore('=') to it.substringAfter('=', "true") },
        if (separator < 0) "" else source.substring(separator + 1),
    )
}

private val decimal = Regex("[+-]?(?:\\d+\\.?\\d*|\\.\\d+)")
private val duration = Regex("[+-]?(?:\\d+\\.?\\d*|\\.\\d+)(?:ms|s)")
private val hex = Regex("(?:[a-fA-F0-9]{3}|[a-fA-F0-9]{4}|[a-fA-F0-9]{6}|[a-fA-F0-9]{8})")

/** Frame values use em units for translations; the renderer applies density/font scaling. */
data class MfmMotion(
    val scaleX: Float = 1f,
    val scaleY: Float = 1f,
    val rotationX: Float = 0f,
    val rotationY: Float = 0f,
    val rotationZ: Float = 0f,
    val x: Float = 0f,
    val y: Float = 0f,
)

fun MfmFunction.motion(phase: Float, reducedMotion: Boolean, inheritedScale: Float): MfmMotion {
    if (name in mfmAnimationDurations && reducedMotion) return MfmMotion()
    val wave = kotlin.math.sin(phase * 2 * kotlin.math.PI).toFloat()
    return when (name) {
        "spin" -> {
            val rotation = phase * 360 * if ("left" in args) -1 else 1
            when {
                "x" in args -> MfmMotion(rotationX = rotation)
                "y" in args -> MfmMotion(rotationY = rotation)
                else -> MfmMotion(rotationZ = rotation)
            }
        }
        "shake", "twitch" -> MfmMotion(
            x = wave * if (name == "shake") .2f else .07f,
            y = (kotlin.math.cos(phase * 14 * kotlin.math.PI).toFloat() - 1) * .07f,
        )
        "jump", "bounce" -> MfmMotion(y = -abs(wave) * .7f)
        "tada" -> MfmMotion(
            rotationZ = wave * 8, scaleX = 1 + abs(wave) * .1f, scaleY = 1 + abs(wave) * .1f,
        )
        "jelly" -> MfmMotion(scaleX = 1 + wave * .15f, scaleY = 1 - wave * .15f)
        "flip" -> MfmMotion(
            scaleX = if ("h" in args || "v" !in args) -1f else 1f,
            scaleY = if ("v" in args) -1f else 1f,
        )
        "scale" -> scale(inheritedScale).let { MfmMotion(scaleX = it.x, scaleY = it.y) }
        "position" -> MfmMotion(x = number("x", 0f, -2f, 2f), y = number("y", 0f, -2f, 2f))
        "rotate" -> MfmMotion(rotationZ = number("deg", 90f, -36000f, 36000f))
        else -> MfmMotion()
    }
}

/** Match the interoperable JavaScript Date range and preserve malformed input. */
fun mfmUnixSeconds(source: String): Long? {
    val raw = source.trim()
    if (!Regex("-?\\d+").matches(raw)) return null
    return raw.toLongOrNull()?.takeIf { it in -8_640_000_000_000L..8_640_000_000_000L }
}

fun mfmFont(args: Map<String, String>): String? =
    listOf("serif", "monospace", "cursive", "fantasy", "emoji", "math").firstOrNull { it in args }
