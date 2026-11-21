package zoo.android

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import zoo.ZooMark

/** Signed-out presentation. Connection settings, persistence and authentication are caller-owned. */
@Composable
internal fun AndroidSignInScreen(
    message: String,
    detailsVisible: Boolean,
    busy: Boolean,
    details: @Composable ColumnScope.() -> Unit,
    actions: @Composable ColumnScope.() -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val dark = colors.background.red < .5f
    Surface(Modifier.fillMaxSize(), color = colors.background) {
        Column(
            Modifier.fillMaxSize().background(
                Brush.verticalGradient(
                    .0f to lerp(colors.background, colors.primary, if (dark) .11f else .18f),
                    .52f to colors.background,
                ),
            ).safeDrawingPadding().imePadding().padding(horizontal = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Column(
                Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState())
                    .padding(vertical = if (detailsVisible) 12.dp else 64.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                ZooMark(size = 76.dp)
                Text("Zoo", style = MaterialTheme.typography.headlineLarge)
                if (message.isNotEmpty()) Text(message, textAlign = TextAlign.Center)
                if (busy) CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
                details()
            }
            Column(
                Modifier.fillMaxWidth().padding(vertical = 12.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(4.dp),
                content = actions,
            )
        }
    }
}
