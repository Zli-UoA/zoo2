package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.font.*
import androidx.compose.ui.unit.*

sealed interface MathNode {
    data class Literal(val value: String) : MathNode

    data class Group(val values: List<MathNode>) : MathNode

    data class Fraction(val top: MathNode, val bottom: MathNode) : MathNode

    data class Root(val value: MathNode) : MathNode

    data class Script(val base: MathNode, val superscript: MathNode?, val subscript: MathNode?) :
        MathNode

    data class Accent(val value: MathNode, val accent: String) : MathNode

    data class Matrix(val rows: List<List<MathNode>>, val brackets: Pair<String, String>) : MathNode
}

private val mathSymbols =
    mapOf(
        "alpha" to "α",
        "beta" to "β",
        "gamma" to "γ",
        "delta" to "δ",
        "epsilon" to "ε",
        "varepsilon" to "ϵ",
        "zeta" to "ζ",
        "eta" to "η",
        "theta" to "θ",
        "vartheta" to "ϑ",
        "iota" to "ι",
        "kappa" to "κ",
        "lambda" to "λ",
        "mu" to "μ",
        "nu" to "ν",
        "xi" to "ξ",
        "pi" to "π",
        "rho" to "ρ",
        "sigma" to "σ",
        "tau" to "τ",
        "upsilon" to "υ",
        "phi" to "φ",
        "varphi" to "ϕ",
        "chi" to "χ",
        "psi" to "ψ",
        "omega" to "ω",
        "Gamma" to "Γ",
        "Delta" to "Δ",
        "Theta" to "Θ",
        "Lambda" to "Λ",
        "Xi" to "Ξ",
        "Pi" to "Π",
        "Sigma" to "Σ",
        "Phi" to "Φ",
        "Psi" to "Ψ",
        "Omega" to "Ω",
        "sum" to "∑",
        "prod" to "∏",
        "int" to "∫",
        "iint" to "∬",
        "oint" to "∮",
        "infty" to "∞",
        "times" to "×",
        "cdot" to "·",
        "div" to "÷",
        "pm" to "±",
        "mp" to "∓",
        "le" to "≤",
        "leq" to "≤",
        "ge" to "≥",
        "geq" to "≥",
        "ne" to "≠",
        "neq" to "≠",
        "approx" to "≈",
        "equiv" to "≡",
        "sim" to "∼",
        "propto" to "∝",
        "to" to "→",
        "rightarrow" to "→",
        "leftarrow" to "←",
        "leftrightarrow" to "↔",
        "Rightarrow" to "⇒",
        "Leftarrow" to "⇐",
        "Leftrightarrow" to "⇔",
        "in" to "∈",
        "notin" to "∉",
        "subset" to "⊂",
        "subseteq" to "⊆",
        "supset" to "⊃",
        "cup" to "∪",
        "cap" to "∩",
        "emptyset" to "∅",
        "forall" to "∀",
        "exists" to "∃",
        "nabla" to "∇",
        "partial" to "∂",
        "ldots" to "…",
        "cdots" to "⋯",
        "vdots" to "⋮",
        "ddots" to "⋱",
        "langle" to "⟨",
        "rangle" to "⟩",
        "lvert" to "|",
        "rvert" to "|",
        "Vert" to "‖",
        "quad" to "　",
        "qquad" to "　　",
        "," to " ",
        ";" to " ",
        "!" to "",
        " " to " ",
    )

class MathParser(private val source: String, private val budget: Int = 0) {
    private var at = 0
    private var depth = 0

    fun parse(): MathNode = if (budget > 16) MathNode.Literal(source) else group(false)

    private fun group(braced: Boolean): MathNode {
        if (++depth > 32) return MathNode.Literal(source.drop(at).also { at = source.length })
        val values = mutableListOf<MathNode>()
        while (at < source.length && (!braced || source[at] != '}')) {
            val c = source[at]
            if (c == '^' || c == '_') {
                at++
                val value = atom()
                val base = values.removeLastOrNull() ?: MathNode.Literal("")
                val previous = base as? MathNode.Script
                values +=
                    MathNode.Script(
                        previous?.base ?: base,
                        if (c == '^') value else previous?.superscript,
                        if (c == '_') value else previous?.subscript,
                    )
            } else values += atom()
        }
        if (braced && at < source.length) at++
        depth--
        return MathNode.Group(values)
    }

    private fun atom(): MathNode {
        if (at >= source.length) return MathNode.Literal("")
        val char = source[at++]
        if (char == '{') return group(true)
        if (char != '\\') return MathNode.Literal(char.toString())
        val start = at
        while (at < source.length && source[at].isLetter()) at++
        if (start == at && at < source.length) at++
        val command = source.substring(start, at)
        fun next(): MathNode {
            while (at < source.length && source[at].isWhitespace()) at++
            return atom()
        }
        return when (command) {
            "frac",
            "dfrac",
            "tfrac",
            "binom" -> MathNode.Fraction(next(), next())
            "sqrt" -> MathNode.Root(next())
            "hat",
            "widehat",
            "bar",
            "overline",
            "vec",
            "dot",
            "ddot",
            "tilde",
            "widetilde",
            "underline" ->
                MathNode.Accent(
                    next(),
                    when (command) {
                        "vec" -> "→"
                        "dot" -> "˙"
                        "ddot" -> "¨"
                        "tilde",
                        "widetilde" -> "~"
                        "bar",
                        "overline",
                        "underline" -> "―"
                        else -> "^"
                    },
                )
            "text",
            "mathrm",
            "mathit",
            "mathbf",
            "mathbb",
            "mathcal",
            "operatorname",
            "displaystyle",
            "textstyle" -> next()
            "left",
            "right",
            "limits",
            "nolimits" -> MathNode.Literal("")
            "begin" -> {
                val environment = source.substring(at).substringAfter('{', "").substringBefore('}')
                if (source.getOrNull(at) == '{')
                    at = source.indexOf('}', at).takeIf { it >= 0 }?.plus(1) ?: at
                val end = source.indexOf("\\end{$environment}", at)
                if (end < 0) MathNode.Literal("\\begin{$environment}")
                else {
                    val body = source.substring(at, end)
                    at = end + environment.length + 6
                    val brackets =
                        when (environment) {
                            "pmatrix" -> "(" to ")"
                            "bmatrix" -> "[" to "]"
                            "Bmatrix" -> "{" to "}"
                            "vmatrix" -> "|" to "|"
                            "Vmatrix" -> "‖" to "‖"
                            "cases" -> "{" to ""
                            else -> "" to ""
                        }
                    MathNode.Matrix(
                        body.split("\\\\").map { row ->
                            row.split('&').map { MathParser(it, budget + 1).parse() }
                        },
                        brackets,
                    )
                }
            }
            else -> MathNode.Literal(mathSymbols[command] ?: command)
        }
    }
}

@Composable
fun MathText(source: String) {
    val node = remember(source) { MathParser(source.take(16_000)).parse() }
    Box(Modifier.horizontalScroll(rememberScrollState()).padding(vertical = 4.dp)) {
        MathLayout(node, 16f)
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
                color = MaterialTheme.colorScheme.onBackground,
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
    MathLayout(remember(source) { MathParser(source.take(16_000)).parse() }, 16f)
}
