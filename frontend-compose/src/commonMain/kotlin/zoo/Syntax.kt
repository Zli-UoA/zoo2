package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.*
import androidx.compose.ui.unit.*

@Composable
fun CodeBlock(source: String, language: String) {
    val dark = LocalApp.current.env.dark
    val colored =
        remember(source, language, dark) {
            val pattern =
                Regex(
                    "(//[^\\n]*|#[^\\n]*|/\\*[\\s\\S]*?\\*/|\"(?:\\\\.|[^\"\\\\])*\"|'(?:\\\\.|[^'\\\\])*'|\\b(?:fun|val|var|class|interface|object|data|sealed|import|package|return|if|else|when|for|while|true|false|null|let|const|function|async|await|def|from|in|not|and|or|try|catch|except|throw|public|private|suspend|fn|impl|pub|use|match|struct|enum|self|SELECT|FROM|WHERE|INSERT|UPDATE|DELETE|CREATE|TABLE)\\b|\\b[0-9]+(?:\\.[0-9]+)?\\b)"
                )
            buildAnnotatedString {
                append(source)
                if (language.isNotEmpty())
                    pattern.findAll(source).forEach { m ->
                        val value = m.value
                        val color =
                            when {
                                value.startsWith("//") ||
                                    value.startsWith('#') ||
                                    value.startsWith("/*") ->
                                    if (dark) Color(0xff8b949e) else Color(0xff6e7781)
                                value.startsWith('"') || value.startsWith('\'') ->
                                    if (dark) Color(0xffa5d6ff) else Color(0xff0a3069)
                                value.first().isDigit() ||
                                    value in setOf("true", "false", "null") ->
                                    if (dark) Color(0xff79c0ff) else Color(0xff0550ae)
                                else -> if (dark) Color(0xffff7b72) else Color(0xffcf222e)
                            }
                        addStyle(SpanStyle(color = color), m.range.first, m.range.last + 1)
                    }
            }
        }
    Box(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .horizontalScroll(rememberScrollState())
            .padding(8.dp)
    ) {
        androidx.compose.material3.Text(
            colored,
            fontFamily = LocalFonts.current.mono,
            fontSize = 13.sp,
            color = MaterialTheme.colorScheme.onBackground,
        )
    }
}
