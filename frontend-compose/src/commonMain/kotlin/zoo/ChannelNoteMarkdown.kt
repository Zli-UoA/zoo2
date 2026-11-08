package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.painter.BitmapPainter
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import com.mikepenz.markdown.compose.components.markdownComponents
import com.mikepenz.markdown.compose.elements.MarkdownCodeBlock
import com.mikepenz.markdown.compose.elements.MarkdownCodeFence
import com.mikepenz.markdown.m3.Markdown
import com.mikepenz.markdown.model.markdownAnnotator
import com.mikepenz.markdown.model.ImageData
import com.mikepenz.markdown.model.ImageTransformer
import org.jetbrains.compose.resources.decodeToImageBitmap
import kotlinx.coroutines.CancellationException
import org.intellij.markdown.MarkdownTokenTypes
import org.intellij.markdown.ast.ASTNode
import org.intellij.markdown.flavours.gfm.GFMFlavourDescriptor
import org.intellij.markdown.flavours.gfm.GFMTokenTypes
import org.intellij.markdown.parser.MarkdownParser

/** The AST must identify the marker; arbitrary bracket text must never be modified. */
internal fun channelNoteMarkerOffset(content: String, start: Int, end: Int): Int? {
    if (start < 0 || end > content.length || end - start < 3) return null
    if (content[start] != '[' || content[start + 2] != ']' || content[start + 1] !in " xX") return null
    if (content.substring(start + 3, end).any { !it.isWhitespace() }) return null
    return start + 1
}

private data class PreparedNoteMarkdown(val text: String, val insertions: List<Int>) {
    fun originalOffset(offset: Int): Int {
        var lo = 0
        var hi = insertions.size
        while (lo < hi) {
            val mid = (lo + hi) / 2
            if (insertions[mid] < offset) lo = mid + 1 else hi = mid
        }
        return offset - 2 * lo
    }
}

/** JetBrains GFM treats empty tasks as plain text. Add an invisible placeholder only
 * after a second parse confirms that the candidate is actually a list checkbox. */
private fun prepareNoteMarkdown(content: String): PreparedNoteMarkdown {
    val candidates = Regex("\\[[ xX]\\][ \\t]*(?=\\r?\\n|$)").findAll(content)
        .map { it.range.first + 3 }.toList()
    if (candidates.isEmpty()) return PreparedNoteMarkdown(content, emptyList())
    fun withInsertions(points: List<Int>): PreparedNoteMarkdown {
        val builder = StringBuilder(content.length + points.size * 2)
        val normalizedPositions = mutableListOf<Int>()
        var previous = 0
        for (point in points) {
            builder.append(content.substring(previous, point))
            normalizedPositions.add(builder.length)
            builder.append(" \u200b")
            previous = point
        }
        builder.append(content.substring(previous))
        return PreparedNoteMarkdown(builder.toString(), normalizedPositions)
    }
    val all = withInsertions(candidates)
    val root = MarkdownParser(GFMFlavourDescriptor()).buildMarkdownTreeFromString(all.text)
    val accepted = mutableSetOf<Int>()
    fun visit(node: ASTNode) {
        if (node.type == GFMTokenTypes.CHECK_BOX) {
            val marker = channelNoteMarkerOffset(all.text, node.startOffset, node.endOffset)
            if (marker != null) accepted.add(all.originalOffset(marker) + 2)
        }
        node.children.forEach(::visit)
    }
    visit(root)
    return withInsertions(candidates.filter { it in accepted })
}

internal fun toggleChannelNoteTask(content: String, offset: Int, checked: Boolean): String {
    if (offset !in content.indices) return content
    val prepared = prepareNoteMarkdown(content)
    val tree = MarkdownParser(GFMFlavourDescriptor()).buildMarkdownTreeFromString(prepared.text)
    fun containsTask(node: ASTNode): Boolean {
        val marker = if (node.type == GFMTokenTypes.CHECK_BOX)
            channelNoteMarkerOffset(prepared.text, node.startOffset, node.endOffset) else null
        return (marker != null && prepared.originalOffset(marker) == offset) || node.children.any(::containsTask)
    }
    if (!containsTask(tree)) return content
    val replacement = if (checked) 'x' else ' '
    if (content[offset] == replacement) return content
    return content.substring(0, offset) + replacement + content.substring(offset + 1)
}

internal fun channelNoteSafeHref(raw: String): Boolean {
    val value = raw.trim()
    return Regex("^(https?:|mailto:)", RegexOption.IGNORE_CASE).containsMatchIn(value) ||
        (value.startsWith("/") && !value.startsWith("//")) ||
        value.startsWith("./") || value.startsWith("../") || value.startsWith("#")
}

private val noteMention = Regex("@[A-Za-z0-9._-]+")

@Composable
fun ChannelNoteMarkdown(app: Session, content: String, canWrite: Boolean, busy: Boolean, onChange: (String) -> Unit) {
    val mentionColor = MaterialTheme.colorScheme.primary
    val annotator = remember(mentionColor) {
        markdownAnnotator { source, node ->
            if (node.type != MarkdownTokenTypes.TEXT) return@markdownAnnotator false
            val text = source.substring(node.startOffset, node.endOffset)
            val matches = noteMention.findAll(text).filter {
                val previous = text.getOrNull(it.range.first - 1)
                previous == null || previous.isWhitespace() || previous in "([{<（「『\"'。、"
            }.toList()
            if (matches.isEmpty()) return@markdownAnnotator false
            var last = 0
            for (match in matches) {
                append(text.substring(last, match.range.first))
                withStyle(SpanStyle(color = mentionColor, fontWeight = FontWeight.Medium)) { append(match.value) }
                last = match.range.last + 1
            }
            append(text.substring(last))
            true
        }
    }
    val codeBlock: @Composable (String) -> Unit = { code ->
        Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceVariant).padding(12.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                IconButton("copy", app.t("channels.notes.copyCode")) {
                    app.run {
                        try {
                            app.platform.invoke("copy", code)
                            app.showToast(app.t("channels.notes.copied"), ToastKind.Success)
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {
                            app.showToast(app.t("channels.notes.copyError"), ToastKind.Error)
                        }
                    }
                }
            }
            Text(code, Modifier.horizontalScroll(rememberScrollState()), fontFamily = FontFamily.Monospace)
        }
    }
    val defaultUriHandler = LocalUriHandler.current
    val safeUriHandler = remember(defaultUriHandler) {
        object : UriHandler {
            override fun openUri(uri: String) {
                if (channelNoteSafeHref(uri)) defaultUriHandler.openUri(uri.trim())
            }
        }
    }
    val imageTransformer = remember(app) {
        object : ImageTransformer {
            @Composable
            override fun transform(link: String): ImageData? {
                val image = produceState<ImageBitmap?>(null, link) {
                    if (!channelNoteSafeHref(link)) return@produceState
                    try { value = app.image(link).decodeToImageBitmap() }
                    catch (e: CancellationException) { throw e }
                    catch (_: Exception) { value = null }
                }.value
                return image?.let { ImageData(painter = BitmapPainter(it)) }
            }
        }
    }
    val prepared = remember(content) { prepareNoteMarkdown(content) }
    CompositionLocalProvider(LocalUriHandler provides safeUriHandler) {
    Markdown(
        content = prepared.text,
        imageTransformer = imageTransformer,
        modifier = Modifier.fillMaxWidth(),
        annotator = annotator,
        components = markdownComponents(
            checkbox = { model ->
                val offset = channelNoteMarkerOffset(model.content, model.node.startOffset, model.node.endOffset)
                val checked = offset != null && model.content[offset].lowercaseChar() == 'x'
                val enabled = canWrite && !busy && offset != null
                val label = model.content.substring(model.node.endOffset).substringBefore('\n')
                    .replace("\u200b", "").trim().ifEmpty { "Todo" }
                Checkbox(
                    checked = checked,
                    modifier = Modifier.then(webControlModifier("checkbox", enabled, checked, attributes = mapOf("aria-label" to label)))
                        .semantics { contentDescription = label },
                    enabled = enabled,
                    onCheckedChange = { value ->
                        if (offset != null) onChange(toggleChannelNoteTask(content, prepared.originalOffset(offset), value))
                    },
                )
            },
            codeFence = { model -> MarkdownCodeFence(model.content, model.node, model.typography.code) { code, _, _ -> codeBlock(code) } },
            codeBlock = { model -> MarkdownCodeBlock(model.content, model.node, model.typography.code) { code, _, _ -> codeBlock(code) } },
        ),
    )
    }
}
