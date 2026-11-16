package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

@Composable
internal fun LoadErrorState(title: String, retryLabel: String, modifier: Modifier = Modifier, retry: () -> Unit) {
    Surface(
        modifier.fillMaxWidth().padding(16.dp).semantics { liveRegion = LiveRegionMode.Polite },
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.errorContainer,
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Icon("cloud-off", modifier = Modifier.size(24.dp), color = MaterialTheme.colorScheme.onErrorContainer)
                Text(title, Modifier.weight(1f), color = MaterialTheme.colorScheme.onErrorContainer)
            }
            Button(retryLabel, variant = "outline", click = retry)
        }
    }
}

@Composable
internal fun TimelineLoadingState(label: String) {
    Column(
        Modifier.fillMaxWidth().padding(16.dp).semantics { liveRegion = LiveRegionMode.Polite },
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
            Text(label, muted = true)
        }
        repeat(3) { MessageLoadingSkeleton() }
    }
}

@Composable
internal fun EmptyContentState(icon: String, title: String, description: String) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 48.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.secondaryContainer) {
            Box(Modifier.size(64.dp), contentAlignment = Alignment.Center) {
                Icon(icon, modifier = Modifier.size(28.dp), color = MaterialTheme.colorScheme.onSecondaryContainer)
            }
        }
        Text(title, weight = FontWeight.SemiBold, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        Text(description, muted = true, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
    }
}
