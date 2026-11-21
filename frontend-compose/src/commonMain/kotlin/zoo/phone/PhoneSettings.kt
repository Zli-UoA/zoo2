package zoo.phone

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*
import zoo.*

val LocalPhoneController = staticCompositionLocalOf<PhoneController?> { null }

private class PhoneDraft(row: Row? = null) {
    var name by mutableStateOf(row?.text("name").orEmpty())
    var shared by mutableStateOf(row?.flag("shared") ?: false)
    var path by mutableStateOf(row?.text("websocket_path") ?: "/sip/ws")
    var domain by mutableStateOf(row?.text("sip_domain").orEmpty())
    var username by mutableStateOf(row?.text("username").orEmpty())
    var password by mutableStateOf("")
    var number by mutableStateOf(row?.text("phone_number").orEmpty())
    val members = mutableStateListOf<String>().apply { addAll(row?.get("member_ids")?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty()) }
    val phones = mutableStateListOf<Row>().apply { addAll(row?.get("phones")?.rows().orEmpty()) }
    fun body() = obj("name" to name.trim(), "shared" to shared, "websocket_path" to path, "sip_domain" to domain, "username" to username, "password" to password, "phone_number" to number, "member_ids" to JsonArray(members.map(::JsonPrimitive)), "phones" to JsonArray(phones))
    fun valid(existing: Boolean) = name.isNotBlank() && name.length <= 100 && path.startsWith("/sip/") && Regex("/[A-Za-z0-9/_.-]+").matches(path) && !path.contains("..") && listOf(domain, username).all { Regex("[A-Za-z0-9._-]{1,128}").matches(it) } && (existing || password.isNotEmpty()) && password.length <= 1024 && (number.isEmpty() || dialNumber(number)) && phones.all { it.text("name").isNotBlank() && it.text("name").length <= 100 && dialNumber(it.text("extension")) }
}
private fun dialNumber(value: String) = Regex("\\+?[0-9]{1,32}").matches(value)

@Composable
fun PhoneSettings(app: Session, controller: PhoneController, users: List<Row>) {
    LaunchedEffect(controller) { if (!controller.loaded) controller.refresh() }
    var editing by remember { mutableStateOf<PhoneDraft?>(null) }
    var editingId by remember { mutableStateOf<String?>(null) }
    var deleting by remember { mutableStateOf<Row?>(null) }
    var number by remember { mutableStateOf("") }
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(app.t("androidPhone.description"), muted = true)
        SettingsSection(app.t("androidPhone.nativeCalls"), app.t("androidPhone.nativeHelp")) {
            Check(app.t("androidPhone.receiveDm"), controller.dmWaiting, enabled = !controller.busy) { controller.receiveDmCalls(it) }
            if (controller.dmWaitingError) PhoneErrorText(app.t("androidPhone.callError"))
        }
        CssFlowRow(horizontalGap = 8.dp, verticalGap = 8.dp) {
            Button(app.t("sip.refresh"), enabled = !controller.loading, variant = "outline") { controller.refresh() }
            if (controller.enabled) Button(app.t("sip.addLine"), enabled = !controller.writing) { editingId = null; editing = PhoneDraft() }
        }
        if (controller.loading) Text(app.t("sip.loading"), Modifier.semantics { liveRegion = LiveRegionMode.Polite })
        if (controller.loadError) PhoneErrorText(app.t("sip.loadError"))
        if (!controller.loading && !controller.loadError && !controller.enabled) Text(app.t("sip.disabled"))
        if (controller.registration.lineId != null) {
            SettingsSection(controller.lines.find { it.text("id") == controller.registration.lineId }?.text("name").orEmpty()) {
                Text(app.t("sip.status.${if (controller.registration.status == "ready") "ready" else "connecting"}"))
                Field(number, app.t("sip.number"), modifier = Modifier.fillMaxWidth(), inputRole = "tel", maxLength = 33) { number = it }
                CssFlowRow(horizontalGap = 8.dp, verticalGap = 8.dp) {
                    Button(app.t("sip.dial"), enabled = controller.registration.status == "ready" && controller.call == null && !controller.busy && dialNumber(number)) { controller.dialSip(number) }
                    Button(app.t("sip.disconnect"), variant = "outline") { controller.disconnectSip() }
                }
            }
        }
        if (controller.enabled && controller.lines.isEmpty()) Text(app.t("sip.empty"))
        controller.lines.forEach { line ->
            SettingsSection(line.text("name"), app.t(if (line.flag("shared")) "sip.shared" else "sip.personal") + line.text("phone_number").let { if (it.isBlank()) "" else " · $it" }) {
                CssFlowRow(horizontalGap = 8.dp, verticalGap = 8.dp) {
                    if (line.flag("can_connect")) Button(app.t("sip.connect"), enabled = controller.registration.status == "offline" && !controller.busy) { controller.connectSip(line.text("id")) }
                    if (line.flag("can_manage")) {
                        Button(app.t("sip.edit"), enabled = !controller.writing, variant = "outline") { editingId = line.text("id"); editing = PhoneDraft(line) }
                        Button(app.t("sip.delete"), enabled = !controller.writing, destructive = true) { deleting = line }
                    }
                }
                if (controller.sipConnectionError == line.text("id")) PhoneErrorText(app.t("sip.callError"))
                line["phones"]?.rows().orEmpty().forEach { phone ->
                    CssFlowRow(horizontalGap = 8.dp, verticalGap = 8.dp) {
                        Text("${phone.text("name")} · ${phone.text("extension")}")
                        Button(app.t("sip.dial"), enabled = controller.registration.lineId == line.text("id") && controller.registration.status == "ready" && controller.call == null && !controller.busy, variant = "outline") { controller.dialSip(phone.text("extension")) }
                    }
                }
            }
        }
        editing?.let { draft ->
            SettingsSection(app.t(if (editingId == null) "sip.addLine" else "sip.edit")) {
                Field(draft.name, app.t("sip.name"), modifier = Modifier.fillMaxWidth(), enabled = !controller.writing, maxLength = 100) { draft.name = it }
                if (app.config.isAdmin || draft.shared) Check(app.t("sip.shared"), draft.shared, enabled = editingId == null && app.config.isAdmin && !controller.writing) { draft.shared = it; draft.members.clear() }
                Field(draft.number, app.t("sip.phone_number"), modifier = Modifier.fillMaxWidth(), enabled = !controller.writing, maxLength = 33) { draft.number = it }
                Field(draft.domain, app.t("sip.sip_domain"), modifier = Modifier.fillMaxWidth(), enabled = !controller.writing, maxLength = 128) { draft.domain = it }
                Field(draft.username, app.t("sip.username"), modifier = Modifier.fillMaxWidth(), enabled = !controller.writing, maxLength = 128) { draft.username = it }
                Field(draft.password, app.t("sip.password"), modifier = Modifier.fillMaxWidth(), secret = true, placeholder = if (editingId != null) app.t("sip.keepPassword") else null, enabled = !controller.writing, maxLength = 1024) { draft.password = it }
                if (draft.shared) {
                    Text(app.t("sip.members")); Text(app.t("sip.membersHelp"), small = true, muted = true)
                    val available = users.filter { !it.flag("is_bot") && !it.flag("is_deactivated") }
                    available.forEach { user -> Check(user.text("display_name").ifBlank { user.text("user_name") }, user.text("id") in draft.members, enabled = !controller.writing) { checked -> if (checked) draft.members.add(user.text("id")) else draft.members.remove(user.text("id")) } }
                    draft.members.filter { id -> available.none { it.text("id") == id } }.forEach { id -> Check("${app.t("sip.unavailableMember")} · $id", true, enabled = !controller.writing) { draft.members.remove(id) } }
                }
                Text(app.t("sip.phones")); Text(app.t("sip.phonesHelp"), small = true, muted = true)
                draft.phones.toList().forEachIndexed { index, phone ->
                    SettingsSection("") {
                        Field(phone.text("name"), app.t("sip.phoneName"), modifier = Modifier.fillMaxWidth(), enabled = !controller.writing, maxLength = 100) { draft.phones[index] = JsonObject(phone + obj("name" to it)) }
                        Field(phone.text("extension"), app.t("sip.extension"), modifier = Modifier.fillMaxWidth(), enabled = !controller.writing, maxLength = 33) { draft.phones[index] = JsonObject(phone + obj("extension" to it)) }
                        Button(app.t("sip.removePhone"), variant = "outline", enabled = !controller.writing) { draft.phones.removeAt(index) }
                    }
                }
                Button(app.t("sip.addPhone"), variant = "outline", enabled = !controller.writing && draft.phones.size < 100) { draft.phones.add(obj("name" to "", "extension" to "")) }
                Text(app.t("sip.pbxHelp"), small = true, muted = true)
                if (controller.writeError) PhoneErrorText(app.t("sip.writeError"))
                CssFlowRow(horizontalGap = 8.dp, verticalGap = 8.dp) {
                    Button(app.t(if (controller.writing) "sip.saving" else "sip.save"), enabled = !controller.writing && draft.valid(editingId != null)) { controller.save(editingId, draft.body()) { editing = null } }
                    Button(app.t("sip.cancel"), variant = "outline", enabled = !controller.writing) { draft.password = ""; editing = null }
                }
            }
        }
    }
    deleting?.let { line ->
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { if (!controller.writing) deleting = null },
            title = { Text(app.t("sip.deleteTitle")) },
            text = { Column { Text(app.t("sip.deleteHelp")); if (controller.writeError) PhoneErrorText(app.t("sip.writeError")) } },
            confirmButton = { Button(app.t("sip.delete"), destructive = true, enabled = !controller.writing) { controller.remove(line.text("id")) { deleting = null } } },
            dismissButton = { Button(app.t("sip.cancel"), variant = "outline", enabled = !controller.writing) { deleting = null } },
        )
    }
}

@Composable
fun PhoneCallBanner(app: Session, controller: PhoneController) {
    val call = controller.call
    if (call == null) {
        if (controller.error) {
            SettingsSection(
                title = "",
                modifier = Modifier.padding(horizontal = AndroidDesign.screenInset),
            ) {
                PhoneErrorText(app.t("androidPhone.callError"))
                Button(app.t("common.close"), variant = "outline", click = controller::clearError)
            }
        }
        return
    }
    SettingsSection(call.label.ifBlank { app.t("sip.title") }, app.t("sip.status.${call.status}") + call.peer.let { if (it.isBlank()) "" else " · $it" }) {
        CssFlowRow(horizontalGap = 8.dp, verticalGap = 8.dp) {
            if (call.status == "incoming") Button(app.t("sip.answer"), enabled = !controller.busy) { controller.answer() }
            Button(app.t("sip.hangup"), destructive = true) { controller.hangup() }
            if (call.status == "active") {
                Button(app.t(if (call.muted) "sip.unmute" else "sip.mute"), enabled = !controller.busy, variant = "outline") { controller.mute() }
                Button(app.t("androidPhone.speaker"), selected = call.speaker, variant = "outline") { controller.speaker() }
            }
        }
        if (call.kind == "sip" && call.status == "active") {
            var tone by remember(call.id) { mutableStateOf("") }
            Field(tone, app.t("sip.dtmf"), maxLength = 1, modifier = Modifier.fillMaxWidth()) { tone = it }
            Button(app.t("sip.sendTone"), enabled = !controller.busy && Regex("[0-9*#]").matches(tone), variant = "outline") { controller.dtmf(tone) }
        }
        if (controller.error) PhoneErrorText(app.t("androidPhone.callError"))
    }
}

@Composable
private fun PhoneErrorText(message: String) {
    Text(
        message,
        Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        color = MaterialTheme.colorScheme.error,
    )
}
