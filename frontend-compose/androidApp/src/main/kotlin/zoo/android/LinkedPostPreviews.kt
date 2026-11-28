package zoo.android

import androidx.compose.runtime.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.debounce
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import zoo.*
import zoo.android.ui.*

/** One account-scoped request for the post IDs represented by composed timeline rows. */
internal fun quoteHandle(userName: String, authorityUrl: String): String? {
    val name = userName.trim().removePrefix("@").takeIf { it.isNotBlank() } ?: return null
    if ('@' in name) return "@$name"
    val host = zoo.publicUrl(authorityUrl)?.host ?: return null
    return "@$name@$host"
}

internal class LinkedPostPreviews(private val fetch: suspend (List<String>) -> List<Row>, private val base: String) : PostPreviewSource {
    private val references = mutableStateMapOf<String, Int>()
    private val previews = mutableStateMapOf<String, PostPreview>()
    var revision by mutableIntStateOf(0)
        private set
    val active get() = references.isNotEmpty()
    override fun watch(id: String) { references[id] = (references[id] ?: 0) + 1 }
    override fun release(id: String) {
        val remaining = (references[id] ?: return) - 1
        if (remaining == 0) references.remove(id) else references[id] = remaining
    }
    override fun read(id: String) = previews[id] ?: PostPreview.Loading
    override fun retry(id: String) { previews.remove(id); revision++ }
    fun invalidate() { revision++ }

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
                    previews[id] = if (row == null || row.present("deleted_at")) PostPreview.Unavailable else PostPreview.Loaded(
                        row.text("display_name").ifBlank { row.text("user_name") }, row.text("content"),
                        timelineAvatarUrl(base, row), row.array("media").rows().mapNotNull { timelinePostMedia(base, it) },
                        quoteHandle(row.text("user_name"), if (row.flag("remote")) row.text("user_id") else base),
                    )
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { batch.filter(references::containsKey).forEach { previews[it] = PostPreview.Failed } }
        }
    }

}

@Composable
internal fun rememberPostPreviews(session: Session, enabled: Boolean): PostPreviewSource {
    val previews = remember(session) {
        LinkedPostPreviews({ ids -> session.api.request("/api/home/entries", "POST", obj(
            "timeline" to "social", "entries" to JsonArray(ids.map(::JsonPrimitive)),
        )).rows() }, session.config.apiBaseUrl)
    }
    DisposableEffect(previews, enabled, previews.active) {
        val stop = if (enabled && previews.active) session.homeLive.subscribe(previews::invalidate) else null
        onDispose { stop?.invoke() }
    }
    LaunchedEffect(previews, enabled) {
        if (enabled) { previews.invalidate(); previews.observe() }
    }
    return previews
}
