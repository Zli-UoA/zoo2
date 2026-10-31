package zoo

import kotlinx.serialization.json.*
import org.jetbrains.compose.resources.decodeToImageBitmap

typealias Row = JsonObject

val json = Json { ignoreUnknownKeys = true }

fun Row.text(key: String): String = (get(key) as? JsonPrimitive)?.contentOrNull.orEmpty()

fun Row.flag(key: String): Boolean = text(key) in setOf("true", "1", "t")

fun Row.present(key: String): Boolean =
    get(key) != null && get(key) != JsonNull && text(key).isNotEmpty()

fun rowKey(row: Row): String =
    row.text("id").ifEmpty { "${row.text("user_id")}:${row.text("channel_id")}" }

fun author(row: Row): String =
    row.text("display_name").ifEmpty { row.text("user_name") }.ifEmpty { row.text("user_id") }

fun visibleChannels(rows: List<Row>): List<Row> =
    rows.filterNot { it.present("deleted_at") }.sortedBy { it.text("channel_name").lowercase() }

fun visibleMessages(rows: List<Row>, parent: String? = null): List<Row> =
    rows
        .filter { !it.present("deleted_at") && it.text("parent_id").ifEmpty { null } == parent }
        .sortedWith(
            compareBy(
                { timestampMillis(it.text("created_at")) ?: Long.MIN_VALUE },
                { it.text("id") },
            )
        )

fun canManage(channel: Row, userId: String, isAdmin: Boolean) =
    isAdmin || channel.text("created_user_id") == userId

fun isMember(rows: List<Row>, channelId: String, userId: String) =
    rows.any { it.text("channel_id") == channelId && it.text("user_id") == userId }

interface Platform {
    fun imageFrames(
        bytes: ByteArray
    ): kotlinx.coroutines.flow.Flow<androidx.compose.ui.graphics.ImageBitmap> =
        kotlinx.coroutines.flow.flow { emit(bytes.decodeToImageBitmap()) }

    fun fileTarget(id: String, bounds: String, receive: (String) -> Boolean) {}

    fun forgetFileTarget(id: String) {}

    fun controlSemantics(id: String, role: String, enabled: Boolean, selected: Boolean?) {}

    fun forgetControlSemantics(id: String) {}

    suspend fun image(url: String): ByteArray? = null

    suspend fun headers(): Map<String, String>

    suspend fun readCache(key: String): String?

    suspend fun writeCache(key: String, value: String)

    fun uuid(): String

    fun now(): String

    fun navigate(path: String)

    fun replace(path: String) = navigate(path)

    fun translate(key: String, args: Map<String, String> = emptyMap()): String

    fun action(name: String)

    fun time(timestamp: String): String

    fun day(timestamp: String): String

    fun previousDay(timestamp: String): String =
        day(
            kotlinx.datetime.Instant.fromEpochMilliseconds(
                    (timestampMillis(timestamp) ?: 0L) - 86_400_000
                )
                .toString()
        )

    suspend fun invoke(name: String, payload: String = ""): String = "{}"
}

data class Config(
    val apiBaseUrl: String,
    val shapeProxyPath: String,
    val userId: String,
    val isAdmin: Boolean,
    val shapeMode: String = "proxy",
    val electricShapeUrl: String = "",
    val txidMode: String = "awaitTxId",
    val assetsBaseUrl: String = "",
    val previewUsers: List<Row> = emptyList(),
    val summalyUrl: String = "",
    val mediaProxyUrl: String = "",
)

data class Environment(
    val path: String = "/",
    val online: Boolean = true,
    val visible: Boolean = true,
    val dark: Boolean = false,
    val locale: String = "ja",
    val mac: Boolean = false,
    val touch: Boolean = false,
    val oidc: Boolean = false,
    val reducedMotion: Boolean = false,
    val development: Boolean = false,
    val focused: Boolean = true,
    val persistenceStatus: String = "ready",
    val persistenceError: String = "",
    val theme: String = "system",
)

fun endpoint(base: String, path: String): String = base.trimEnd('/') + "/" + path.trimStart('/')

/** Match Rust chars().count(), including supplementary Unicode characters. */
fun String.codePointCount(): Int {
    var result = 0
    var index = 0
    while (index < length) {
        val ch = this[index++]
        if (ch.isHighSurrogate() && index < length && this[index].isLowSurrogate()) index++
        result++
    }
    return result
}

fun timestampMillis(raw: String): Long? =
    try {
        val value = raw.trim().replace(' ', 'T')
        val suffix = value.drop(10)
        kotlinx.datetime.Instant.parse(
                if (suffix.contains('Z') || suffix.contains('+') || suffix.contains('-')) value
                else "${value}Z"
            )
            .toEpochMilliseconds()
    } catch (_: IllegalArgumentException) {
        null
    }

fun groupedWith(previous: Row?, current: Row): Boolean {
    if (previous == null || previous.text("user_id") != current.text("user_id")) return false
    val earlier = timestampMillis(previous.text("created_at")) ?: return false
    val later = timestampMillis(current.text("created_at")) ?: return false
    return later - earlier in 0..300_000
}
