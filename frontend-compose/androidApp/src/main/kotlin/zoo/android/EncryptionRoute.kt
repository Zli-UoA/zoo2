package zoo.android

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.OutlinedTextField
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import zoo.*
import kotlinx.serialization.json.JsonPrimitive
import zoo.android.ui.*
import zoo.crypto.E2eeStatus
import zoo.crypto.UnlockMethod
import zoo.crypto.Unlocked
import zoo.crypto.ffi.SetupStart

@Composable
internal fun EncryptionRoute(
    session: Session,
    platform: AndroidPlatform,
    back: () -> Unit,
) {
    val manager = platform.e2ee
    val setup = manager?.setup
    val store by requireNotNull(manager).readyStore.collectAsState()
    var status by remember { mutableStateOf<E2eeStatus?>(null) }
    var start by remember { mutableStateOf<SetupStart?>(null) }
    var pending by remember { mutableStateOf<Unlocked?>(null) }
    var recovery by remember { mutableStateOf("") }
    var confirmed by remember { mutableStateOf(false) }
    var passkeyAdded by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf(false) }
    var devices by remember { mutableStateOf<List<Row>>(emptyList()) }
    var unsigned by remember { mutableStateOf<Set<String>>(emptySet()) }
    var rename by remember { mutableStateOf<Row?>(null) }
    var deviceName by remember { mutableStateOf("") }
    var remove by remember { mutableStateOf<Row?>(null) }
    var job by remember { mutableStateOf<Job?>(null) }
    val scope = rememberCoroutineScope()
    val reauthenticate = platform.reauthenticate
    val passkey = platform.passkey
    suspend fun reload() {
        val loadedStatus = setup?.loadStatus()
        val loaded = session.api.request("/api/e2ee/devices").rows()
        // 「未署名」の表示のため、自分の自己署名鍵と署名を読む（表示だけ。配布先の判定は署名を検証する）。
        val keys = session.api.request("/api/e2ee/keys/query", "POST",
            obj("user_ids" to kotlinx.serialization.json.JsonArray(listOf(JsonPrimitive(session.config.userId)))))
        unsigned = zoo.crypto.unsignedDevices(keys as? kotlinx.serialization.json.JsonObject ?: obj(),
            session.config.userId, loaded.map { it.text("device_id") })
        devices = loaded
        status = loadedStatus
    }
    fun perform(block: suspend () -> Unit) {
        if (job?.isActive == true) return
        failure = false
        job = scope.launch {
            try { block() }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { failure = true }
            finally { job = null }
        }
    }
    fun unlock(method: UnlockMethod) = perform {
        val unlocked = requireNotNull(setup).unlock(method)
        recovery = ""
        if (unlocked.needsReauth) pending = unlocked
        else {
            unlocked.session.close()
            reload()
        }
    }
    val sensitive = start != null || status is E2eeStatus.Locked
    DisposableEffect(sensitive) {
        val release = if (sensitive) platform.protectScreen() else ({})
        onDispose { release() }
    }
    DisposableEffect(Unit) {
        onDispose {
            job?.cancel()
            start?.session?.close()
            pending?.session?.close()
        }
    }
    LaunchedEffect(store) {
        if (store != null) perform {
            if (withTimeoutOrNull(30_000) { reload(); true } != true) failure = true
        }
    }
    AndroidShell(null, platform.env.dark, {}, platform.env.locale) { colors ->
        PageHeader("エンドツーエンド暗号化", colors, onBack = back)
        Column(Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)) {
            val enabled = job == null && platform.env.online
            val setupStart = start
            val unfinished = pending
            when {
                setup == null || store == null -> Text("この端末の暗号化を準備しています", color = colors.secondary)
                setupStart != null -> {
                    Text("リカバリーキーを安全な場所に控えてください", color = colors.foreground)
                    Text(setupStart.recoveryKey, Modifier.fillMaxWidth().padding(12.dp),
                        color = colors.foreground, fontSize = 18.sp, textAlign = TextAlign.Center)
                    TextAction(if (confirmed) "保存済み" else "キーを控えた", colors, icon = UiIcon.Check, style = TextActionStyle.Neutral) {
                        confirmed = !confirmed
                    }
                    if (passkey != null) TextAction(if (passkeyAdded) "パスキーを追加済み" else "パスキーを追加", colors,
                        enabled && !passkeyAdded, icon = UiIcon.Key, style = TextActionStyle.Neutral) {
                        perform {
                            setup.addSetupPasskey(setupStart.session, passkey.client, passkey.rpId, null)
                            passkeyAdded = true
                        }
                    }
                    if (reauthenticate != null) TextAction("保存", colors, confirmed && enabled, icon = UiIcon.Save) {
                        perform {
                            setup.completeSetup(setupStart.session, reauthenticate())
                            setupStart.session.close()
                            start = null
                            reload()
                        }
                    }
                }
                unfinished != null && reauthenticate != null -> TextAction("初期設定を続ける", colors, enabled, icon = UiIcon.Next) {
                    perform {
                        setup.saveRemaining(unfinished.session, unfinished.secretsRevision, reauthenticate())
                        unfinished.session.close()
                        pending = null
                        reload()
                    }
                }
                status is E2eeStatus.NotSetUp -> TextAction("暗号化を設定", colors, enabled, icon = UiIcon.Lock) {
                    perform {
                        confirmed = false
                        passkeyAdded = false
                        start = setup.begin()
                    }
                }
                status is E2eeStatus.Locked -> {
                    if (passkey != null) TextAction("パスキーで解錠", colors, enabled, icon = UiIcon.Key) {
                        unlock(UnlockMethod.Passkey(passkey.client, passkey.rpId))
                    }
                    Text("リカバリーキー", color = colors.foreground)
                    BasicTextField(recovery, { recovery = it }, Modifier.fillMaxWidth().padding(vertical = 8.dp),
                        enabled = enabled, singleLine = true, visualTransformation = PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
                        textStyle = TextStyle(color = colors.foreground, fontSize = 16.sp),
                        cursorBrush = SolidColor(colors.foreground))
                    TextAction("リカバリーキーで解錠", colors, enabled && recovery.isNotBlank(), icon = UiIcon.Key) {
                        unlock(UnlockMethod.Recovery(recovery))
                    }
                }
                status is E2eeStatus.Unlocked -> {
                    Text("この端末は解錠済みです", color = colors.foreground)
                    TextAction("鍵をバックアップ", colors, enabled, icon = UiIcon.Upload, style = TextActionStyle.Neutral) {
                        perform {
                            if (setup.uploadKeyBackup() is zoo.crypto.BackupUploadResult.Conflict) failure = true
                        }
                    }
                    TextAction("鍵のバックアップを復元", colors, enabled, icon = UiIcon.Download, style = TextActionStyle.Neutral) {
                        perform { setup.importKeyBackup() }
                    }
                }
            }
            if (job != null) TextAction("キャンセル", colors, icon = UiIcon.Close, style = TextActionStyle.Neutral) { job?.cancel() }
            if (failure) Text("暗号化の操作を完了できませんでした", color = colors.foreground)
            if (failure || (status == null && setup != null && store != null && job == null)) {
                TextAction("状態を読み直す", colors, enabled, icon = UiIcon.Refresh, style = TextActionStyle.Neutral) { perform { reload() } }
            }
            if (devices.isNotEmpty()) {
                Text(session.t("e2ee.devices.title"), color = colors.foreground, fontWeight = AndroidWeights.label)
                Text(session.t("e2ee.devices.description"), color = colors.secondary, fontSize = 14.sp)
                devices.forEach { device ->
                    val id = device.text("device_id")
                    val current = id == manager.currentDeviceId
                    Column(Modifier.fillMaxWidth()) {
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp),
                            verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                            TextAction(device.text("display_name").ifBlank { session.t("e2ee.devices.unnamed") }, colors, enabled,
                                style = TextActionStyle.Plain, modifier = Modifier.weight(1f), icon = UiIcon.Format) {
                                deviceName = device.text("display_name")
                                rename = device
                            }
                            if (current) Text(session.t("e2ee.devices.current"), color = colors.secondary, fontSize = 14.sp)
                            else TextAction(session.t("e2ee.devices.delete"), colors, enabled && manager.currentDeviceId != null,
                                icon = UiIcon.Delete, style = TextActionStyle.Neutral) { remove = device }
                        }
                        val details = listOfNotNull(
                            id.takeIf { id in unsigned }?.let { session.t("e2ee.devices.unsigned") },
                            device.text("created_at").takeIf { it.isNotBlank() }?.let {
                                session.t("e2ee.devices.created", "date" to "${platform.day(it)} ${platform.time(it)}")
                            },
                            device.text("last_seen_at").takeIf { it.isNotBlank() }?.let {
                                session.t("e2ee.devices.lastSeen", "date" to "${platform.day(it)} ${platform.time(it)}")
                            },
                            id,
                        )
                        Text(details.joinToString(" · "), Modifier.padding(start = 4.dp), color = colors.secondary, fontSize = 12.sp)
                    }
                }
                Text(session.t("e2ee.devices.deleteIsNotRevocation"), color = colors.secondary, fontSize = 14.sp)
            }
            val revocation = manager.revocation
            if (status is E2eeStatus.Unlocked && revocation != null) {
                RevocationSection(session, platform, revocation, manager.currentDeviceId, colors, enabled) {
                    scope.launch { try { reload() } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { } }
                }
            } else if (revocation != null && status is E2eeStatus.Locked) {
                Text(session.t("e2ee.revocation.needsUnlock"), color = colors.secondary, fontSize = 14.sp)
            }
            val sas = manager.sas
            if (status is E2eeStatus.Unlocked && sas != null) {
                // 他のユーザーの検証：暗号化チャンネル（削除されていないもの）を共有する相手（Web の `DevicesSettings`）。
                val channels by session.repo.channels.rows.collectAsState()
                val members by session.repo.members.rows.collectAsState()
                val users by session.repo.users.rows.collectAsState()
                val peers = remember(channels, members, users) {
                    val encrypted = channels.filter { it.flag("encrypted") && !it.present("deleted_at") }.map { it.text("id") }.toSet()
                    val ids = members.filter { it.text("channel_id") in encrypted }.map { it.text("user_id") }
                        .filter { it != session.config.userId }.distinct()
                    ids.map { id -> id to (users.firstOrNull { it.text("id") == id }?.let(::author)?.ifBlank { null } ?: id) }
                        .sortedBy { it.second }
                }
                Text(session.t("e2ee.verification.title"), color = colors.foreground, fontWeight = AndroidWeights.label)
                Text(session.t("e2ee.verification.description"), color = colors.secondary, fontSize = 14.sp)
                if (peers.isEmpty()) Text(session.t("e2ee.verification.empty"), color = colors.secondary, fontSize = 14.sp)
                peers.forEach { (id, name) ->
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp),
                        verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Text(name, Modifier.weight(1f), color = colors.foreground)
                        TextAction(session.t("e2ee.sas.start"), colors, enabled, icon = UiIcon.Check, style = TextActionStyle.Neutral) {
                            perform { sas.request(id) }
                        }
                    }
                }
            }
        }
    }
    rename?.let { device ->
        AlertDialog(
            onDismissRequest = { rename = null },
            title = { Text(session.t("e2ee.devices.rename")) },
            text = { OutlinedTextField(deviceName, { deviceName = it }, singleLine = true) },
            confirmButton = {
                DialogAction("保存", UiIcon.Save, enabled = deviceName.isNotBlank() && deviceName.codePointCount() <= 100) {
                    val name = deviceName.trim()
                    rename = null
                    perform {
                        session.api.request("/api/e2ee/devices/${device.text("device_id").encodeURLParameter()}", "PATCH",
                            obj("display_name" to name))
                        reload()
                    }
                }
            },
            dismissButton = { DialogAction("キャンセル", UiIcon.Close) { rename = null } },
        )
    }
    remove?.let { device ->
        AlertDialog(
            onDismissRequest = { remove = null },
            title = { Text(session.t("e2ee.devices.deleteConfirmTitle")) },
            text = { Text(listOf(device.text("display_name").ifBlank { session.t("e2ee.devices.unnamed") },
                session.t("e2ee.devices.deleteConfirmDescription")).joinToString("\n\n")) },
            confirmButton = {
                DialogAction(session.t("e2ee.devices.deleteOnly"), UiIcon.Delete) {
                    remove = null
                    perform {
                        check(device.text("device_id") != manager.currentDeviceId)
                        session.api.request("/api/e2ee/devices/${device.text("device_id").encodeURLParameter()}", "DELETE")
                        reload()
                    }
                }
            },
            dismissButton = { DialogAction("キャンセル", UiIcon.Close) { remove = null } },
        )
    }
}
