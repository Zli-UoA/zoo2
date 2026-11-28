package zoo.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
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
    data class Choice(override val id: String, override val title: String, val selected: Boolean, val description: String? = null) : SettingsEntry
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
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        for (group in groups) {
            val choices = group.entries.any { it is SettingsEntry.Choice }
            Column(if (choices) Modifier.selectableGroup() else Modifier) {
                if (group.title.isNotBlank()) Text(
                    group.title,
                    Modifier.padding(start = 16.dp, end = 16.dp, bottom = 4.dp),
                    color = colors.secondary,
                    fontSize = 13.sp,
                    fontWeight = AndroidWeights.label,
                )
                for (entry in group.entries) {
                    val action = when (entry) {
                        is SettingsEntry.Information -> Modifier
                        is SettingsEntry.Choice -> Modifier.selectable(entry.selected, role = Role.RadioButton) { open(entry.id) }
                        is SettingsEntry.Destination -> Modifier.clickable(role = Role.Button) { open(entry.id) }
                        is SettingsEntry.Toggle -> Modifier.toggleable(entry.enabled, role = Role.Switch) { toggle(entry.id, it) }
                    }
                    val detail = when (entry) {
                        is SettingsEntry.Choice -> entry.description
                        is SettingsEntry.Destination -> entry.value
                        else -> null
                    }
                    LabeledRow(entry.title, detail, colors, action) {
                        when (entry) {
                            is SettingsEntry.Information -> Unit
                            is SettingsEntry.Choice -> if (entry.selected) UiGlyph(UiIcon.Check, colors.foreground, iconSize = AndroidControlSize.inlineIcon)
                            is SettingsEntry.Destination -> UiGlyph(UiIcon.Next, colors.secondary, iconSize = AndroidControlSize.inlineIcon)
                            is SettingsEntry.Toggle -> Switch(checked = entry.enabled, onCheckedChange = null)
                        }
                    }
                    if (entry is SettingsEntry.Toggle && entry.id == "swipe" && entry.enabled) {
                        var tried by rememberSaveable { mutableStateOf(false) }
                        Row(Modifier.fillMaxWidth().then(swipeReplyGesture { tried = true })
                            .padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            UiGlyph(UiIcon.Bubble, colors.secondary, AndroidControlSize.inlineIcon)
                            Text(if (tried) "返信ジェスチャーを認識しました" else "ここを左にスワイプして試す",
                                color = colors.secondary, fontSize = 14.sp)
                        }
                    }
                }
            }
        }
    }
}
