package zoo.android

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Checkbox
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import zoo.Session
import zoo.author
import zoo.text
import zoo.android.ui.*
import zoo.crypto.AuthzRevocationStep
import zoo.crypto.AuthzRevocationUrls
import zoo.crypto.E2eeRevocation
import zoo.crypto.RevocationDevice
import zoo.crypto.RevocationException
import zoo.crypto.RevocationResult
import zoo.crypto.RevocationResume
import zoo.crypto.UnlockMethod
import zoo.crypto.ffi.ResignNote
import zoo.crypto.ffi.RevocationScope
import zoo.crypto.ffi.RevocationStage
import zoo.crypto.ffi.RevocationSummary
import zoo.crypto.notResignedUsers

/** 失効の画面の段（Web の `RevocationDialog` と同じ順）。 */
private sealed interface RevocationStep {
    data object Idle : RevocationStep
    data class Select(val devices: List<RevocationDevice>) : RevocationStep
    /** SSSS を開け直す。`resume` なら止まった失効を続ける。 */
    data class Unlock(val scope: RevocationScope, val deleted: List<String>, val resume: Boolean) : RevocationStep
    data class Review(val summary: RevocationSummary, val recoveryKey: String?) : RevocationStep
    data class Running(val stage: RevocationStage) : RevocationStep
    data class Done(val result: RevocationResult) : RevocationStep
    data object Conflict : RevocationStep
}

private fun stageKey(stage: RevocationStage) = "e2ee.revocation.stage." + stage.name.lowercase()

private fun noteKey(note: ResignNote) = "e2ee.revocation.note." + note.name.lowercase()

/**
 * 端末と鍵の失効（docs/e2ee.md の「失効の手順」「失効と検証の実装（#362）」。Web の `DevicesSettings` の失効）。
 * 削除する端末と範囲を選ぶ、AuthZ のセッションとサービス接続を止める、SSSS を開け直す、（全体なら）新しい
 * リカバリーキーを控える、署名し直さない相手を確かめる、再認証して実行する、の順。
 */
@Composable
internal fun RevocationSection(
    session: Session,
    platform: AndroidPlatform,
    revocation: E2eeRevocation,
    currentDeviceId: String?,
    colors: AndroidColors,
    enabled: Boolean,
    /** 解錠しているか。解錠していなくても、終わっていない失効は続けられる（Web の `DevicesSettingsView` と同じ）。 */
    unlocked: Boolean,
    onFinished: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val pending by revocation.summary.collectAsState()
    val users by session.repo.users.rows.collectAsState()
    var step by remember { mutableStateOf<RevocationStep>(RevocationStep.Idle) }
    var busy by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf(false) }
    var selected by remember { mutableStateOf(setOf<String>()) }
    var fullScope by remember { mutableStateOf(true) }
    var authzDone by remember { mutableStateOf(false) }
    var recovery by remember { mutableStateOf("") }
    var keyNoted by remember { mutableStateOf(false) }
    fun t(key: String, vararg args: Pair<String, String>) = session.t(key, *args)
    // AuthZ の画面は issuer と同じ origin にある（Web の `accountRevocationUrls`）。
    val links = platform.issuer?.let { AuthzRevocationUrls.fromIssuer(it) }
    LaunchedEffect(Unit) { try { revocation.status() } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { } }
    DisposableEffect(Unit) { onDispose { revocation.dispose() } }
    fun perform(block: suspend () -> Unit) {
        if (busy) return
        busy = true
        failed = false
        scope.launch {
            try { block() }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: RevocationException.Conflict) { step = RevocationStep.Conflict }
            catch (_: RevocationException.ServerKeyMismatch) { step = RevocationStep.Conflict }
            catch (_: Exception) {
                failed = true
                // 実行の途中で止まったら（通信の失敗、再認証の取り消しなど）、「失効を続ける」に戻す。
                if (step is RevocationStep.Running) {
                    try { revocation.status() } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { }
                    step = RevocationStep.Idle
                }
            }
            finally { busy = false }
        }
    }
    @Composable fun body(text: String) = Text(text, color = colors.secondary, fontSize = 14.sp)
    @Composable fun heading(text: String) = Text(text, color = colors.foreground, fontWeight = AndroidWeights.label)
    @Composable fun authzSteps(steps: List<AuthzRevocationStep>, titleKey: String) {
        heading(t(titleKey))
        steps.forEach { body("・" + t(it.translationKey)) }
        body(t("e2ee.authz.note"))
        if (links != null) {
            TextAction(t("e2ee.authz.open.security"), colors, true, TextActionStyle.Neutral, icon = UiIcon.External) {
                platform.action("open:${links.security}")
            }
            TextAction(t("e2ee.authz.open.services"), colors, true, TextActionStyle.Neutral, icon = UiIcon.External) {
                platform.action("open:${links.services}")
            }
        }
    }
    val can = enabled && !busy
    Column(Modifier.fillMaxWidth().testTag("revocation"), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        heading(t("e2ee.revocation.title"))
        when (val current = step) {
            RevocationStep.Idle -> {
                body(t("e2ee.revocation.description"))
                val unfinished = pending
                if (unfinished != null) {
                    heading(t("e2ee.revocation.pendingTitle"))
                    body(t("e2ee.revocation.pendingDescription"))
                    TextAction(t("e2ee.revocation.resume"), colors, can, icon = UiIcon.Next) {
                        recovery = ""
                        step = RevocationStep.Unlock(unfinished.scope, unfinished.deletedDevices, resume = true)
                    }
                } else if (!unlocked) body(t("e2ee.revocation.needsUnlock"))
                else TextAction(t("e2ee.revocation.start"), colors, can, TextActionStyle.Neutral, icon = UiIcon.Key) {
                    perform {
                        selected = emptySet()
                        authzDone = false
                        step = RevocationStep.Select(revocation.loadDevices(currentDeviceId))
                    }
                }
            }
            is RevocationStep.Select -> {
                heading(t("e2ee.revocation.selectTitle"))
                body(t("e2ee.revocation.selectDescription"))
                current.devices.forEach { device ->
                    Row(Modifier.fillMaxWidth().clickable(enabled = !device.current) {
                        selected = if (device.deviceId in selected) selected - device.deviceId else selected + device.deviceId
                    }, verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(device.deviceId in selected, null, enabled = !device.current)
                        Column(Modifier.weight(1f)) {
                            Text(device.displayName?.ifBlank { null } ?: t("e2ee.devices.unnamed"), color = colors.foreground)
                            Text(listOfNotNull(
                                if (device.current) t("e2ee.devices.current") else null,
                                if (!device.signed) t("e2ee.devices.unsigned") else null,
                                t("e2ee.devices.created", "date" to "${platform.day(device.createdAt)} ${platform.time(device.createdAt)}"),
                                device.lastSeenAt?.let { t("e2ee.devices.lastSeen", "date" to "${platform.day(it)} ${platform.time(it)}") },
                                device.deviceId,
                            ).joinToString(" · "), color = colors.secondary, fontSize = 12.sp)
                        }
                    }
                }
                heading(t("e2ee.revocation.scopeTitle"))
                for (full in listOf(true, false)) {
                    val key = if (full) "full" else "device_only"
                    Row(Modifier.fillMaxWidth().clickable { fullScope = full }, verticalAlignment = Alignment.CenterVertically) {
                        RadioButton(fullScope == full, null)
                        Column(Modifier.weight(1f)) {
                            Text(t("e2ee.revocation.scope.$key"), color = colors.foreground)
                            body(t("e2ee.revocation.scopeDescription.$key"))
                        }
                    }
                }
                body(t("e2ee.revocation.irreversible"))
                authzSteps(AuthzRevocationStep.BEFORE, "e2ee.authz.beforeTitle")
                Row(Modifier.fillMaxWidth().clickable { authzDone = !authzDone }, verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(authzDone, null)
                    Text(t("e2ee.authz.acknowledge"), color = colors.foreground)
                }
                TextAction(t("e2ee.revocation.next"), colors, can && authzDone, icon = UiIcon.Next) {
                    recovery = ""
                    step = RevocationStep.Unlock(if (fullScope) RevocationScope.FULL else RevocationScope.DEVICE_ONLY,
                        selected.toList(), resume = false)
                }
                TextAction(t("e2ee.sas.cancel"), colors, can, TextActionStyle.Neutral, icon = UiIcon.Close) { step = RevocationStep.Idle }
            }
            is RevocationStep.Unlock -> {
                body(t(if (current.resume) "e2ee.revocation.unlockResume" else "e2ee.revocation.unlockDescription"))
                fun open(method: UnlockMethod) = perform {
                    revocation.openSecretStorage(method)
                    recovery = ""
                    if (current.resume) {
                        when (val resumed = revocation.resume()) {
                            is RevocationResume.Resumed -> step = RevocationStep.Review(resumed.summary, null)
                            is RevocationResume.Restart -> {
                                // 全体の失効を SSSS の保存の前で止めた。resume が残した SSSS で、同じ範囲と端末で始め直す。
                                // 記録は捨てたので、始め直しに失敗したら、解錠し直して新しい失効として始める。
                                step = RevocationStep.Unlock(resumed.scope, resumed.deletedDevices, resume = false)
                                val summary = revocation.begin(resumed.scope, resumed.deletedDevices)
                                keyNoted = false
                                step = RevocationStep.Review(summary, if (resumed.scope == RevocationScope.FULL) revocation.newRecoveryKey() else null)
                            }
                            is RevocationResume.Conflict -> step = RevocationStep.Conflict
                        }
                    } else {
                        revocation.deleteSelectedDevices(current.deleted)
                        val summary = revocation.begin(current.scope, current.deleted)
                        keyNoted = false
                        step = RevocationStep.Review(summary, if (current.scope == RevocationScope.FULL) revocation.newRecoveryKey() else null)
                    }
                }
                platform.passkey?.let { passkey ->
                    TextAction(t("e2ee.unlockWithPasskey"), colors, can, icon = UiIcon.Key) { open(passkey) }
                }
                Text(t("e2ee.recoveryKeyInput"), color = colors.foreground)
                BasicTextField(recovery, { recovery = it }, Modifier.fillMaxWidth().padding(vertical = 8.dp),
                    enabled = can, singleLine = true, visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
                    textStyle = TextStyle(color = colors.foreground, fontSize = 16.sp), cursorBrush = SolidColor(colors.foreground))
                TextAction(t("e2ee.unlockWithRecoveryKey"), colors, can && recovery.isNotBlank(), icon = UiIcon.Key) {
                    open(UnlockMethod.Recovery(recovery))
                }
                TextAction(t("e2ee.sas.cancel"), colors, can, TextActionStyle.Neutral, icon = UiIcon.Close) { step = RevocationStep.Idle }
            }
            is RevocationStep.Review -> {
                val summary = current.summary
                current.recoveryKey?.let { key ->
                    body(t("e2ee.revocation.newRecoveryKey"))
                    Text(key, Modifier.fillMaxWidth().padding(12.dp), color = colors.foreground, fontSize = 18.sp, textAlign = TextAlign.Center)
                    Row(Modifier.fillMaxWidth().clickable { keyNoted = !keyNoted }, verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(keyNoted, null)
                        Text(t("e2ee.recoveryKeyConfirm"), color = colors.foreground)
                    }
                    platform.passkey?.let { passkey ->
                        body(t("e2ee.revocation.passkeyDescription"))
                        TextAction(t("e2ee.addPasskey"), colors, can, TextActionStyle.Neutral, icon = UiIcon.Key) {
                            perform { revocation.addPasskey(passkey.client, passkey.rpId, null) }
                        }
                    }
                    body(t("e2ee.revocation.fullWarning"))
                }
                val notes = summary.notResignedUsers()
                if (notes.isNotEmpty()) {
                    heading(t("e2ee.revocation.notesTitle"))
                    body(t("e2ee.revocation.notesDescription"))
                    notes.forEach { note ->
                        val name = users.firstOrNull { it.text("id") == note.userId }?.let(::author)?.ifBlank { null } ?: note.userId
                        body("・$name：" + note.reasons.joinToString("、") { t(noteKey(it)) })
                    }
                }
                if (summary.versionsWithoutKey.isNotEmpty()) {
                    body(t("e2ee.revocation.versionsWithoutKey", "versions" to summary.versionsWithoutKey.joinToString(", ")))
                }
                body(t("e2ee.revocation.executeDescription"))
                TextAction(t("e2ee.revocation.execute"), colors, can && (current.recoveryKey == null || keyNoted), icon = UiIcon.Check) {
                    perform {
                        step = RevocationStep.Running(summary.stage)
                        val result = revocation.continueRevocation(null, platform.reauthenticate?.let { re -> { re() } },
                            summary.revocationId) { progress -> step = RevocationStep.Running(progress.stage) }
                        step = RevocationStep.Done(result)
                        onFinished()
                    }
                }
            }
            is RevocationStep.Running -> body(t(stageKey(current.stage)))
            is RevocationStep.Done -> {
                Text(t("e2ee.revocation.done"), color = colors.foreground)
                val result = current.result
                if (result.droppedSessions > 0) {
                    body(session.tCount("e2ee.revocation.droppedSessions", result.droppedSessions.toInt()))
                }
                if (result.unsignedDevices.isNotEmpty()) {
                    heading(t("e2ee.revocation.unsignedTitle"))
                    body(t("e2ee.revocation.unsignedDescription"))
                    result.unsignedDevices.forEach { body("・" + (it.displayName?.ifBlank { null } ?: t("e2ee.devices.unnamed")) + "（${it.deviceId}）") }
                }
                authzSteps(AuthzRevocationStep.AFTER, "e2ee.authz.afterTitle")
                TextAction(t("e2ee.revocation.finish"), colors, can, TextActionStyle.Neutral) { step = RevocationStep.Idle }
            }
            RevocationStep.Conflict -> {
                body(t("e2ee.revocation.conflict"))
                TextAction(t("e2ee.revocation.restart"), colors, can, icon = UiIcon.Refresh) {
                    perform {
                        revocation.abandon()
                        step = RevocationStep.Idle
                    }
                }
            }
        }
        if (failed) Text(t("e2ee.errors.revocationFailed"), color = colors.foreground, fontSize = 14.sp)
    }
}
