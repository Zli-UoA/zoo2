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

@Composable
internal fun AndroidSettingsIndex(app: Session, destinations: List<Pair<String, String>>, onOpen: (String) -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        SettingsIndexHeading(app.t("settings.appearance"))
        SettingsIndexRow(app.t("theme.title"), app.t("theme.${app.env.theme}"), "palette") { onOpen("appearance") }
        SettingsIndexHeading(app.t("settings.interaction"))
        SettingsIndexSwitch(app.t("settings.swipeReply"), "reply", app.env.swipeReply) {
            app.platform.action("toggleSwipeReply")
        }
        SettingsIndexSwitch(app.t("settings.operationHaptics"), "activity", app.env.operationHaptics) {
            app.platform.action("toggleOperationHaptics")
        }
        SettingsIndexHeading(app.t("settings.zooTitle"))
        for ((destination, title) in destinations.filter { it.first != "appearance" }) {
            SettingsIndexRow(title, null, when (destination) {
                "sip" -> "message-circle-more"
                "encryption" -> "lock"
                "members" -> "users"
                "emojis" -> "smile-plus"
                "apps" -> "bot"
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
