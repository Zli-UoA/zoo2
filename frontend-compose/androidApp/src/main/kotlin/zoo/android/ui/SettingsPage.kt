package zoo.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class SettingsGroup(val title: String, val entries: List<SettingsEntry>)

internal sealed interface SettingsEntry {
    val id: String
    val title: String

    data class Destination(override val id: String, override val title: String, val value: String? = null) : SettingsEntry
    data class Information(override val id: String, override val title: String) : SettingsEntry
    data class Toggle(override val id: String, override val title: String, val enabled: Boolean) : SettingsEntry
}

@Composable
internal fun SettingsPage(
    groups: List<SettingsGroup>,
    colors: AndroidColors,
    open: (String) -> Unit,
    toggle: (String, Boolean) -> Unit,
    title: String = "設定",
    onBack: (() -> Unit)? = null,
) {
    PageHeader(title, colors, onBack = onBack)
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        for (group in groups) {
            Column {
                Text(
                    group.title,
                    Modifier.padding(bottom = 4.dp),
                    color = colors.secondary,
                    fontSize = 13.sp,
                    fontWeight = AndroidWeights.label,
                )
                for (entry in group.entries) {
                    val action = when (entry) {
                        is SettingsEntry.Information -> Modifier
                        is SettingsEntry.Destination -> Modifier.clickable(role = Role.Button) { open(entry.id) }
                        is SettingsEntry.Toggle -> Modifier.toggleable(entry.enabled, role = Role.Switch) { toggle(entry.id, it) }
                    }
                    Row(
                        Modifier.fillMaxWidth().then(action).heightIn(min = AndroidControlSize.target).padding(vertical = 8.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column(Modifier.weight(1f)) {
                            Text(entry.title, color = colors.foreground, fontSize = 16.sp)
                            if (entry is SettingsEntry.Destination && entry.value != null) {
                                Text(entry.value, color = colors.secondary, fontSize = 13.sp)
                            }
                        }
                        when (entry) {
                            is SettingsEntry.Information -> Unit
                            is SettingsEntry.Destination -> UiGlyph(UiIcon.Next, colors.secondary)
                            is SettingsEntry.Toggle -> Switch(checked = entry.enabled, onCheckedChange = null)
                        }
                    }
                }
            }
        }
    }
}
