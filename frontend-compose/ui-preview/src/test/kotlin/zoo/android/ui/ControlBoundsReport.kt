package zoo.android.ui

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.junit4.ComposeTestRule
import java.io.File
import kotlin.math.roundToInt

/** Record visible click nodes in the merged semantics tree, including disabled controls. */
internal fun ComposeTestRule.recordControlBounds(sample: String) {
    val rows = onAllNodes(hasClickAction()).fetchSemanticsNodes().mapNotNull { node ->
        val visible = node.boundsInRoot
        if (visible.isEmpty) return@mapNotNull null
        val config = node.config
        val label = config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString(" ")
            ?: config.getOrNull(SemanticsProperties.Text)?.joinToString(" ") { it.text }
            ?: config.getOrNull(SemanticsProperties.TestTag).orEmpty()
        val scale = node.layoutInfo.density.density
        fun dp(px: Float) = (px * 10 / scale).roundToInt() / 10.0
        val touch = node.touchBoundsInRoot
        listOf(sample, label.replace(Regex("\\s+"), " "), SemanticsProperties.Disabled !in config,
            dp(node.size.width.toFloat()), dp(node.size.height.toFloat()),
            dp(visible.width), dp(visible.height), dp(touch.width), dp(touch.height),
            dp(visible.left), dp(visible.top), dp(touch.left), dp(touch.top),
            generateSequence(node) { it.parent }.last().id).joinToString("\t")
    }
    val directory = File("build/reports/control-bounds").apply { mkdirs() }
    directory.resolve("$sample.tsv").writeText(rows.joinToString("\n", postfix = if (rows.isEmpty()) "" else "\n"))
}
