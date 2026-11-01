package zoo

import kotlinx.serialization.json.*
import kotlinx.coroutines.CancellationException

internal fun homePreferenceKey(app: Session, prefix: String): String? =
    app.config.userId.takeIf { it.isNotEmpty() }?.let {
        prefix + JsonArray(listOf(JsonPrimitive(app.config.apiBaseUrl), JsonPrimitive(it))).toString()
    }

internal suspend fun readHomePreference(app: Session, key: String?): String {
    if (key == null) return ""
    return try {
        app.platform.invoke("storageRead", key)
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        ""
    }
}

internal suspend fun writeHomePreference(app: Session, key: String?, value: String) {
    if (key == null) return
    try {
        app.platform.invoke("storageWrite", obj("key" to key, "value" to value).toString())
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        // Posting remains available when browser storage is unavailable.
    }
}

internal fun parseHomePostSettings(raw: String): Row {
    val value = runCatching { Json.parseToJsonElement(raw) as? JsonObject }.getOrNull()
    val visibility = value?.text("visibility")?.takeIf { it in listOf("public", "unlisted", "followers") } ?: "unlisted"
    val apub = (value?.get("apub") as? JsonPrimitive)?.takeUnless { it.isString }?.booleanOrNull ?: true
    val channel = (value?.get("channel_id") as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull
        ?.takeIf { it.isNotEmpty() && it.length <= 200 && visibility != "followers" }
    return obj("visibility" to visibility, "apub" to apub, "channel_id" to channel)
}

internal fun homeReplyVisibility(current: String, parent: String): String = when {
    parent == "followers" -> "followers"
    parent == "unlisted" && current == "public" -> "unlisted"
    else -> current
}
