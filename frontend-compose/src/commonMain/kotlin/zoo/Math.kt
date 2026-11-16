package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.font.*
import androidx.compose.ui.unit.*

@Composable
fun MathText(source: String) {
    val node = remember(source) { MathParser(source.take(16_000)).parse() }
    Box(Modifier.horizontalScroll(rememberScrollState()).padding(vertical = 4.dp)) {
        MathLayout(node, MaterialTheme.typography.bodyMedium.merge(LocalMfmStyle.current).fontSize.value)
    }
}

@Composable
private fun MathLayout(node: MathNode, size: Float) {
    when (node) {
        is MathNode.Literal ->
            androidx.compose.material3.Text(
                node.value,
                fontFamily =
                    if (node.value.all { it.isLetter() }) LocalFonts.current.mathItalic
                    else LocalFonts.current.math,
                fontSize = size.sp,
                color = LocalMfmStyle.current.color.takeIf { it != androidx.compose.ui.graphics.Color.Unspecified }
                    ?: MaterialTheme.colorScheme.onBackground,
            )
        is MathNode.Group ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                node.values.forEach { MathLayout(it, size) }
            }
        is MathNode.Fraction ->
            Column(
                Modifier.width(IntrinsicSize.Max).padding(horizontal = 2.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Box(Modifier.padding(horizontal = 3.dp, vertical = 1.dp)) {
                    MathLayout(node.top, size * .9f)
                }
                HorizontalDivider(thickness = 1.dp)
                Box(Modifier.padding(horizontal = 3.dp, vertical = 1.dp)) {
                    MathLayout(node.bottom, size * .9f)
                }
            }
        is MathNode.Root ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("√", size = (size * 1.3f).sp)
                Column {
                    HorizontalDivider()
                    MathLayout(node.value, size)
                }
            }
        is MathNode.Script ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                MathLayout(node.base, size)
                Column {
                    node.superscript?.let { MathLayout(it, size * .7f) }
                    node.subscript?.let { MathLayout(it, size * .7f) }
                }
            }
        is MathNode.Accent ->
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(node.accent, size = (size * .7f).sp)
                MathLayout(node.value, size)
            }
        is MathNode.Matrix ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(node.brackets.first, size = (size * node.rows.size.coerceAtMost(4)).sp)
                Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    node.rows.forEach { row ->
                        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                            row.forEach { MathLayout(it, size) }
                        }
                    }
                }
                Text(node.brackets.second, size = (size * node.rows.size.coerceAtMost(4)).sp)
            }
    }
}

@Composable
fun MathInline(source: String) {
    MathLayout(
        remember(source) { MathParser(source.take(16_000)).parse() },
        MaterialTheme.typography.bodyMedium.merge(LocalMfmStyle.current).fontSize.value,
    )
}
