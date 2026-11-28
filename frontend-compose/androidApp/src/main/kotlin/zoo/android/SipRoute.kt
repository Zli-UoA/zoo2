package zoo.android

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.serialization.json.*
import zoo.*
import zoo.android.ui.*
import zoo.phone.PhoneController

private class PhoneDraft(row: Row? = null) {
    var name by mutableStateOf(row?.text("name").orEmpty())
    var shared by mutableStateOf(row?.flag("shared") ?: false)
    var path by mutableStateOf(row?.text("websocket_path") ?: "/sip/ws")
    var domain by mutableStateOf(row?.text("sip_domain").orEmpty())
    var username by mutableStateOf(row?.text("username").orEmpty())
    var password by mutableStateOf("")
    var number by mutableStateOf(row?.text("phone_number").orEmpty())
    val members = mutableStateListOf<String>().apply {
        addAll(row?.array("member_ids")?.map { it.jsonPrimitive.content }.orEmpty())
    }
    val phones = mutableStateListOf<Row>().apply { addAll(row?.array("phones")?.rows().orEmpty()) }

    fun body() = obj(
        "name" to name.trim(), "shared" to shared, "websocket_path" to path,
        "sip_domain" to domain, "username" to username, "password" to password, "phone_number" to number,
        "member_ids" to JsonArray(members.map(::JsonPrimitive)), "phones" to JsonArray(phones),
    )

    fun valid(existing: Boolean): Boolean =
        name.isNotBlank() && name.length <= 100 && path.startsWith("/sip/") &&
            Regex("/[A-Za-z0-9/_.-]+").matches(path) && !path.contains("..") &&
            listOf(domain, username).all { Regex("[A-Za-z0-9._-]{1,128}").matches(it) } &&
            (existing || password.isNotEmpty()) && password.length <= 1024 &&
            (number.isEmpty() || dialNumber(number)) && phones.all {
                val label = it.text("name")
                label.isNotBlank() && label.length <= 100 && dialNumber(it.text("extension"))
            }
}
private fun dialNumber(value: String) = Regex("\\+?[0-9]{1,32}").matches(value)

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun SipRoute(app: Session, controller: PhoneController, users: List<Row>, back: () -> Unit) {
    val online = app.env.online
    var refreshStarted by remember(controller, online) { mutableStateOf(false) }
    LaunchedEffect(controller, online) {
        if (online) {
            controller.refresh(force = true)
            refreshStarted = true
        }
    }
    val canManageLines = refreshStarted && !controller.loading && !controller.loadError && !controller.writing
    var draft by remember { mutableStateOf<PhoneDraft?>(null) }
    var editingId by remember { mutableStateOf<String?>(null) }
    var deleting by remember { mutableStateOf<Row?>(null) }
    var number by remember { mutableStateOf("") }
    val canDial = controller.registration.status == "ready" && controller.call == null && !controller.busy
    AndroidShell(null, app.env.dark, {}, app.env.locale) { colors ->
        PageHeader(app.t("sip.title"), colors, onBack = back) {
            UiIconButton(UiIcon.Refresh, app.t("sip.refresh"), colors.foreground,
                { controller.refresh() }, enabled = online && !controller.loading)
            if (controller.enabled) UiIconButton(UiIcon.Add, app.t("sip.addLine"), colors.foreground,
                {
                    editingId = null
                    draft = PhoneDraft()
                }, enabled = canManageLines)
        }
        Column(
            Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            when {
                controller.loading -> Text(app.t("sip.loading"), color = colors.secondary)
                controller.loadError -> SipError(app.t("sip.loadError"))
                !controller.enabled -> Text(app.t("sip.disabled"), color = colors.secondary)
                controller.lines.isEmpty() -> Text(app.t("sip.empty"), color = colors.secondary)
            }
            if (controller.registration.lineId != null) {
                val status = if (controller.registration.status == "ready") "ready" else "connecting"
                Text(app.t("sip.status.$status"), color = colors.foreground)
                SipField(colors, number, app.t("sip.number"), 33, true, KeyboardType.Phone) { number = it }
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextAction(app.t("sip.dial"), colors, canDial && dialNumber(number), icon = UiIcon.Phone) {
                        controller.dialSip(number)
                    }
                    TextAction(app.t("sip.disconnect"), colors, style = TextActionStyle.Neutral) { controller.disconnectSip() }
                }
            }
            for (line in controller.lines) {
                val id = line.text("id")
                Text(line.text("name"), color = colors.foreground, fontSize = 16.sp, fontWeight = AndroidWeights.emphasis)
                Text(listOf(app.t(if (line.flag("shared")) "sip.shared" else "sip.personal"), line.text("phone_number"))
                    .filter { it.isNotBlank() }.joinToString(" · "), color = colors.secondary, fontSize = 13.sp)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (line.flag("can_connect")) TextAction(app.t("sip.connect"), colors,
                        controller.registration.status == "offline" && !controller.busy, icon = UiIcon.Phone) {
                        controller.connectSip(id)
                    }
                    if (line.flag("can_manage")) {
                        TextAction(app.t("sip.edit"), colors, canManageLines, TextActionStyle.Plain, icon = UiIcon.Format) {
                            editingId = id
                            draft = PhoneDraft(line)
                        }
                        TextAction(app.t("sip.delete"), colors, canManageLines, TextActionStyle.Plain, icon = UiIcon.Delete) {
                            deleting = line
                        }
                    }
                }
                if (controller.sipConnectionError == id) SipError(app.t("sip.callError"))
                for (contact in line.array("phones").rows()) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text("${contact.text("name")} · ${contact.text("extension")}", Modifier.weight(1f), color = colors.foreground)
                        UiIconButton(UiIcon.Phone, app.t("sip.dial"), colors.secondary, {
                            controller.dialSip(contact.text("extension"))
                        }, enabled = controller.registration.lineId == id && canDial)
                    }
                }
                Spacer(Modifier.height(8.dp))
            }
            draft?.let { editor ->
                val editable = !controller.writing
                Text(app.t(if (editingId == null) "sip.addLine" else "sip.edit"), color = colors.foreground,
                    fontSize = 18.sp, fontWeight = AndroidWeights.emphasis)
                SipField(colors, editor.name, app.t("sip.name"), 100, editable) { editor.name = it }
                if (app.config.isAdmin || editor.shared) {
                    SipToggle(app.t("sip.shared"), editor.shared,
                        editable && editingId == null && app.config.isAdmin, colors) {
                        editor.shared = it
                        editor.members.clear()
                    }
                }
                SipField(colors, editor.number, app.t("sip.phone_number"), 33, editable, KeyboardType.Phone) { editor.number = it }
                SipField(colors, editor.domain, app.t("sip.sip_domain"), 128, editable) { editor.domain = it }
                SipField(colors, editor.username, app.t("sip.username"), 128, editable) { editor.username = it }
                SipField(colors, editor.password, app.t(if (editingId == null) "sip.password" else "sip.keepPassword"),
                    1024, editable, KeyboardType.Password) { editor.password = it }
                if (editor.shared) {
                    Text(app.t("sip.membersHelp"), color = colors.secondary)
                    val available = users.filterNot { it.flag("is_bot") || it.flag("is_deactivated") }
                    for (user in available) {
                        val id = user.text("id")
                        SipToggle(author(user), id in editor.members, editable, colors) {
                            if (it) editor.members.add(id) else editor.members.remove(id)
                        }
                    }
                    for (id in editor.members.filter { current -> available.none { it.text("id") == current } }) {
                        SipToggle("${app.t("sip.unavailableMember")} · $id", true, editable, colors) { editor.members.remove(id) }
                    }
                }
                Text(app.t("sip.phonesHelp"), color = colors.secondary)
                editor.phones.toList().forEachIndexed { index, contact ->
                    SipField(colors, contact.text("name"), app.t("sip.phoneName"), 100, editable) {
                        editor.phones[index] = JsonObject(editor.phones[index] + obj("name" to it))
                    }
                    SipField(colors, contact.text("extension"), app.t("sip.extension"), 33, editable, KeyboardType.Phone) {
                        editor.phones[index] = JsonObject(editor.phones[index] + obj("extension" to it))
                    }
                    TextAction(app.t("sip.removePhone"), colors, editable, TextActionStyle.Plain, icon = UiIcon.Delete) {
                        editor.phones.removeAt(index)
                    }
                }
                TextAction(app.t("sip.addPhone"), colors, editable && editor.phones.size < 100, TextActionStyle.Neutral, icon = UiIcon.Add) {
                    editor.phones.add(obj("name" to "", "extension" to ""))
                }
                Text(app.t("sip.pbxHelp"), color = colors.secondary)
                if (controller.writeError) SipError(app.t("sip.writeError"))
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextAction(app.t(if (editable) "sip.save" else "sip.saving"), colors,
                        editable && editor.valid(editingId != null), icon = UiIcon.Save) {
                        controller.save(editingId, editor.body()) { draft = null }
                    }
                    TextAction(app.t("sip.cancel"), colors, editable, TextActionStyle.Neutral) {
                        editor.password = ""
                        draft = null
                    }
                }
            }
        }
        deleting?.let { line ->
            AlertDialog(
                onDismissRequest = { if (!controller.writing) deleting = null },
                title = { Text(app.t("sip.deleteTitle")) },
                text = {
                    Column {
                        Text(app.t("sip.deleteHelp"))
                        if (controller.writeError) SipError(app.t("sip.writeError"))
                    }
                },
                confirmButton = {
                    DialogAction(app.t("sip.delete"), UiIcon.Delete, enabled = !controller.writing) {
                        controller.remove(line.text("id")) { deleting = null }
                    }
                },
                dismissButton = {
                    DialogAction(app.t("sip.cancel"), UiIcon.Close, enabled = !controller.writing) { deleting = null }
                },
            )
        }
    }
}

@Composable
private fun SipField(
    colors: AndroidColors,
    value: String, label: String, limit: Int, enabled: Boolean,
    keyboard: KeyboardType = KeyboardType.Text, change: (String) -> Unit,
) {
    LabeledField(label, value, colors, enabled = enabled, keyboard = KeyboardOptions(keyboardType = keyboard),
        transformation = if (keyboard == KeyboardType.Password) PasswordVisualTransformation() else VisualTransformation.None) {
        if (it.length <= limit) change(it)
    }
}

@Composable
private fun SipToggle(label: String, checked: Boolean, enabled: Boolean, colors: AndroidColors, change: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
            .toggleable(checked, enabled = enabled, role = Role.Switch, onValueChange = change),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, Modifier.weight(1f), color = colors.foreground)
        Switch(checked, onCheckedChange = null, enabled = enabled)
    }
}

@Composable
private fun SipError(label: String) {
    Text(label, Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        color = androidx.compose.material3.MaterialTheme.colorScheme.error)
}
