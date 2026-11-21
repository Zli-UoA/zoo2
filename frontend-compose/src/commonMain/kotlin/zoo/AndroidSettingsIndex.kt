package zoo

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

internal data class AndroidSettingsLabels(
    val appearance: String,
    val theme: String,
    val themeValue: String,
    val interaction: String,
    val swipeReply: String,
    val haptics: String,
    val options: String,
)

@Composable
internal fun AndroidSettingsIndex(
    labels: AndroidSettingsLabels,
    destinations: List<Pair<String, String>>,
    swipeReply: Boolean,
    haptics: Boolean,
    onOpen: (String) -> Unit,
    onToggleSwipeReply: () -> Unit,
    onToggleHaptics: () -> Unit,
) {
    Column(Modifier.fillMaxWidth()) {
        SettingsIndexHeading(labels.appearance)
        SettingsIndexRow(labels.theme, labels.themeValue, "palette") { onOpen("appearance") }
        SettingsIndexHeading(labels.interaction)
        SettingsIndexSwitch(labels.swipeReply, "reply", swipeReply, onToggleSwipeReply)
        SettingsIndexSwitch(labels.haptics, "activity", haptics, onToggleHaptics)
        SettingsIndexHeading(labels.options)
        for ((destination, title) in destinations.filter { it.first != "appearance" }) {
            SettingsIndexRow(title, null, when (destination) {
                "sip" -> "message-circle-more"
                "encryption" -> "lock"
                "members" -> "users"
                "emojis" -> "smile-plus"
                "apps" -> "bot"
                "android-notifications" -> "bell"
                "android-update" -> "refresh-cw"
                else -> "settings"
            }) { onOpen(destination) }
        }
    }
}

@Composable
internal fun AndroidAppearanceSettings(app: Session) {
    SettingRow(app.t("theme.title"), icon = "palette") { ThemeToggle(app) }
    SettingsIndexDivider()
    SettingRow(app.t("language.title"), icon = "languages") { LanguageSelect(app) }
}

@Composable
private fun SettingsIndexHeading(text: String) {
    Text(text, Modifier.padding(top = AndroidDesign.spaceLarge, bottom = AndroidDesign.spaceSmall),
        small = true, muted = true, weight = FontWeight.SemiBold)
}

@Composable
private fun SettingsIndexRow(title: String, detail: String?, icon: String, open: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = AndroidDesign.touchTarget)
            .clickable(role = Role.Button, onClick = open).padding(vertical = AndroidDesign.spaceMedium),
        horizontalArrangement = Arrangement.spacedBy(AndroidDesign.spaceLarge),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, modifier = Modifier.size(20.dp))
        Column(Modifier.weight(1f)) {
            Text(title, weight = FontWeight.SemiBold)
            detail?.let { Text(it, small = true, muted = true) }
        }
        Icon("chevron-right", modifier = Modifier.size(16.dp))
    }
    SettingsIndexDivider()
}

@Composable
private fun SettingsIndexSwitch(label: String, icon: String, value: Boolean, onToggle: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = AndroidDesign.touchTarget),
        horizontalArrangement = Arrangement.spacedBy(AndroidDesign.spaceLarge),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, modifier = Modifier.size(20.dp))
        Text(label, Modifier.weight(1f), weight = FontWeight.SemiBold)
        Switch(value, onCheckedChange = { onToggle() }, modifier = Modifier.semantics { contentDescription = label })
    }
    SettingsIndexDivider()
}

@Composable
private fun SettingsIndexDivider() = HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
