package zoo.android

import androidx.compose.runtime.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.debounce
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import zoo.*
import zoo.android.ui.*

internal fun quoteHandle(userName: String, authorityUrl: String): String? {
    val name = userName.trim().removePrefix("@").takeIf { it.isNotBlank() } ?: return null
    if ('@' in name) return "@$name"
    val host = zoo.publicUrl(authorityUrl)?.host ?: return null
    return "@$name@$host"
}

/** Active rows share one request. Only small public previews can survive a short scroll away. */
internal class LinkedPostPreviews(private val fetch: suspend (List<String>) -> List<Row>, private val base: String,
    private val idleLimit: Int = 8, private val now: () -> Long = { System.nanoTime() / 1_000_000 }) : PostPreviewSource {
    private data class Cached(val preview: PostPreview.Loaded, val expires: Long)
    private val idle = LinkedHashMap<String, Cached>()
    private val expires = mutableMapOf<String, Long>()
    private val references = mutableStateMapOf<String, Int>()
    private val previews = mutableStateMapOf<String, PostPreview>()
    var revision by mutableIntStateOf(0)
        private set
    val active get() = references.isNotEmpty()
    override fun watch(id: String) {
        references[id] = (references[id] ?: 0) + 1
        idle.remove(id)?.takeIf { it.expires > now() }?.let {
            previews[id] = it.preview
            expires[id] = it.expires
        }
    }
    override fun release(id: String) {
        val remaining = (references[id] ?: return) - 1
        if (remaining > 0) {
            references[id] = remaining
            return
        }
        references.remove(id)
        val preview = previews.remove(id)
        val deadline = expires.remove(id)
        if (idleLimit > 0 && preview is PostPreview.Loaded && deadline != null && deadline > now()) {
            idle[id] = Cached(preview, deadline)
            while (idle.size > idleLimit) idle.remove(idle.keys.first())
        }
    }
    override fun read(id: String) = previews[id] ?: PostPreview.Loading
    override fun retry(id: String) { previews.remove(id); revision++ }
    fun invalidate() {
        idle.clear()
        expires.clear()
        revision++
    }

    fun suspendDisplay() {
        invalidate()
        previews.clear()
    }

    @OptIn(kotlinx.coroutines.FlowPreview::class)
    suspend fun observe() {
        var appliedRevision = -1
        snapshotFlow { references.keys.toSet() to revision }.debounce(60).collect { (ids, version) ->
            previews.keys.toList().filterNot(ids::contains).forEach(previews::remove)
            refresh(ids, version != appliedRevision, version)
            appliedRevision = version
        }
    }

    internal suspend fun refresh(ids: Set<String>, invalidate: Boolean, version: Int = revision) {
        val pending = if (invalidate) ids else ids - previews.keys
        for (batch in pending.chunked(100)) {
            try {
                val results = fetch(batch).map { it.nested("post").ifEmpty { it } }.associateBy { it.text("id") }
                for (id in batch) {
                    if (id !in references || version != revision) continue
                    val row = results[id]
                    expires.remove(id)
                    if (row != null && row.text("visibility") == "public" && !row.present("deleted_at") &&
                        !row.present("ciphertext") && !row.present("quote") && !row.present("parent") &&
                        !row.flag("sensitive") && row.array("media").rows().none { it.flag("is_sensitive") } &&
                        row.toString().length <= 8192) expires[id] = now() + 30_000
                    previews[id] = if (row == null || row.present("deleted_at")) PostPreview.Unavailable else PostPreview.Loaded(
                        row.text("display_name").ifBlank { row.text("user_name") }, row.text("content"),
                        timelineAvatarUrl(base, row), row.array("media").rows().mapNotNull { timelinePostMedia(base, it) },
                        quoteHandle(row.text("user_name"), if (row.flag("remote")) row.text("user_id") else base),
                    )
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) {
                if (version == revision) batch.filter(references::containsKey).forEach { previews[it] = PostPreview.Failed }
            }
        }
    }

}

@Composable
internal fun rememberPostPreviews(session: Session, enabled: Boolean, limit: Int): PostPreviewSource {
    val previews = remember(session, limit, session.navigationCacheRevision) {
        LinkedPostPreviews({ ids -> session.api.request("/api/home/entries", "POST", obj(
            "timeline" to "social", "entries" to JsonArray(ids.map(::JsonPrimitive)),
        )).rows() }, session.config.apiBaseUrl, limit)
    }
    DisposableEffect(previews, enabled, previews.active) {
        val stop = if (enabled && previews.active) session.homeLive.subscribe(previews::invalidate) else null
        onDispose { stop?.invoke() }
    }
    LaunchedEffect(previews, enabled) {
        if (enabled) { previews.invalidate(); previews.observe() } else previews.suspendDisplay()
    }
    return previews
}
