package zoo.crypto

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout
import zoo.Button
import zoo.Field
import zoo.Session
import zoo.SettingsSection
import zoo.Text
import zoo.crypto.ffi.SetupStart
import zoo.crypto.ffi.ZooCryptoException

/**
 * 設定の「暗号化」タブ（#360）で使う、アプリ側の機能。React の `EncryptionSettings` の Android 版。
 */
class E2eeSettingsHost(
    val setup: E2eeSetup,
    /** パスキーの RP ID（ログインに使う issuer のホスト名）。使えなければ `null`（リカバリーキーだけ）。 */
    val rpId: String?,
    /** Credential Manager。使えない端末では `null`。 */
    val passkeys: PasskeyClient?,
    /** 直前の再認証（Custom Tabs と App Links）。ID トークンを返す。失敗は [ReauthException]。 */
    val reauthenticate: suspend () -> String,
    /** リカバリーキーをクリップボードに置く（機微な内容として）。 */
    val copySecret: (String) -> Unit,
)

/** 画面の段階。 */
private sealed interface Phase {
    data object Loading : Phase

    /** 端末の登録が終わっていない（暗号ストアを使えない）。 */
    data object Unavailable : Phase

    data class Status(val status: E2eeStatus) : Phase

    /** 初回設定で作った鍵（リカバリーキーを表示する）。 */
    class Created(val start: SetupStart) : Phase

    /** 解錠したが、続きの保存に再認証が要る。 */
    class Continue(val unlocked: Unlocked) : Phase
}

/** 暗号化タブの中身。 */
@Composable
fun EncryptionSettings(app: Session, host: E2eeSettingsHost) {
    val t = { key: String -> app.t("e2ee.$key") }
    val scope = rememberCoroutineScope()
    var phase by remember { mutableStateOf<Phase>(Phase.Loading) }
    var error by remember { mutableStateOf("") }
    var notice by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf<String?>(null) }
    var job by remember { mutableStateOf<Job?>(null) }
    var recoveryInput by remember { mutableStateOf("") }
    var confirmed by remember { mutableStateOf(false) }
    var passkeyAdded by remember { mutableStateOf(false) }
    var revision by remember { mutableStateOf(0) }

    // 画面を閉じたら、メモリーの秘密（マスター鍵の秘密鍵と SSSS 鍵）を捨てる（TUI と同じ）。
    fun discard(current: Phase) {
        when (current) {
            is Phase.Created -> current.start.session.close()
            is Phase.Continue -> current.unlocked.session.close()
            else -> Unit
        }
    }
    DisposableEffect(Unit) { onDispose { discard(phase) } }

    fun replace(next: Phase) {
        val previous = phase
        phase = next
        if (previous !== next) discard(previous)
    }

    LaunchedEffect(revision) {
        replace(Phase.Loading)
        try {
            replace(Phase.Status(withTimeout(30.seconds) { host.setup.loadStatus() }))
        } catch (_: TimeoutCancellationException) {
            replace(Phase.Unavailable)
        } catch (failure: CancellationException) {
            throw failure
        } catch (_: Exception) {
            replace(Phase.Unavailable)
            error = t("errors.loadFailed")
        }
    }

    /** 状態を読み直す。表示中の誤りは消す（読み直しの理由を示すときは、呼んだあとで入れる）。 */
    fun reload() {
        error = ""
        confirmed = false
        passkeyAdded = false
        recoveryInput = ""
        revision++
    }

    /** 失敗を表示する文言にする（秘密やサーバーの本文は出さない）。 */
    fun describe(failure: Throwable): String =
        when (failure) {
            is E2eeSetupException.ReauthRequired -> t("errors.reauthFailed")
            is E2eeSetupException.Conflict -> t("errors.setupConflict")
            is E2eeSetupException.ServerKeyMismatch -> t("errors.mismatch")
            is E2eeSetupException.NoSecretStorage -> t("errors.loadFailed")
            is ReauthException ->
                when (failure.reason) {
                    ReauthException.Reason.TIMEOUT -> t("errors.reauthTimeout")
                    ReauthException.Reason.BROWSER_UNAVAILABLE -> t("android.browserUnavailable")
                    else -> t("errors.reauthFailed")
                }
            is PasskeyException ->
                when (failure.error) {
                    PasskeyError.CANCELLED -> t("errors.passkeyCancelled")
                    PasskeyError.PRF_UNSUPPORTED -> t("errors.passkeyNoPrf")
                    PasskeyError.ORIGIN_NOT_ALLOWED -> t("errors.passkeyOriginNotAllowed")
                    PasskeyError.NO_SLOT -> t("errors.passkeyNoSlot")
                    PasskeyError.UNKNOWN -> t("errors.passkeyCancelled")
                }
            is ZooCryptoException.InvalidRecoveryKey -> t("errors.invalidRecoveryKey")
            is ZooCryptoException.KeySlotUnlock, is ZooCryptoException.NoKeySlot -> t("errors.wrongRecoveryKey")
            else -> t("errors.saveFailed")
        }

    /** 操作を 1 つずつ行う。`label` は進み具合の表示。 */
    fun run(label: String, block: suspend () -> Unit) {
        if (job?.isActive == true) return
        error = ""
        notice = ""
        busy = label
        job =
            scope.launch {
                try {
                    block()
                } catch (failure: CancellationException) {
                    throw failure
                } catch (failure: Exception) {
                    if (failure is E2eeSetupException.Conflict || failure is E2eeSetupException.ServerKeyMismatch) {
                        // 手元の鍵は使えない。状態を読み直す（文言は残す）。
                        val message = describe(failure)
                        reload()
                        error = message
                    } else {
                        error = describe(failure)
                    }
                } finally {
                    busy = null
                    job = null
                }
            }
    }

    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        SettingsSection(t("title"), t("description")) {}
        when (val current = phase) {
            Phase.Loading -> Progress(t("loading"))
            Phase.Unavailable ->
                SettingsSection("", "") {
                    Text(t("unavailable"))
                    Button(t("retry"), variant = "outline") { reload() }
                }
            is Phase.Status ->
                when (val status = current.status) {
                    E2eeStatus.NotSetUp ->
                        SettingsSection(t("notSetUpTitle"), t("notSetUpDescription")) {
                            Button(t("start"), primary = true, enabled = busy == null) {
                                try {
                                    replace(Phase.Created(host.setup.begin()))
                                } catch (failure: Exception) {
                                    error = describe(failure)
                                }
                            }
                        }
                    is E2eeStatus.Locked -> {
                        val description =
                            when (status.reason) {
                                LockReason.NEW -> t("lockedDescription")
                                LockReason.RESUME -> t("resumeDescription")
                                LockReason.MISMATCH -> t("reunlockDescription")
                            }
                        val passkeyReady =
                            host.passkeys != null && host.rpId != null &&
                                remember(status.storage) { passkeySlots(host.setup.userId, status.storage).isNotEmpty() }
                        SettingsSection(t("lockedTitle"), description) {
                            if (passkeyReady) {
                                Button(t("unlockWithPasskey"), primary = true, enabled = busy == null) {
                                    run(t("unlocking")) {
                                        val unlocked = host.setup.unlock(UnlockMethod.Passkey(host.passkeys, host.rpId))
                                        if (unlocked.needsReauth) replace(Phase.Continue(unlocked))
                                        else {
                                            unlocked.session.close()
                                            reload()
                                        }
                                    }
                                }
                            }
                            Field(
                                recoveryInput,
                                t("recoveryKeyInput"),
                                Modifier.fillMaxWidth(),
                                secret = true,
                                enabled = busy == null,
                                placeholder = t("recoveryKeyPlaceholder"),
                            ) { recoveryInput = it }
                            Button(
                                t("unlockWithRecoveryKey"),
                                primary = !passkeyReady,
                                variant = if (passkeyReady) "outline" else "default",
                                enabled = busy == null && recoveryInput.isNotBlank(),
                            ) {
                                val input = recoveryInput
                                run(t("unlocking")) {
                                    val unlocked = host.setup.unlock(UnlockMethod.Recovery(input))
                                    recoveryInput = ""
                                    if (unlocked.needsReauth) replace(Phase.Continue(unlocked))
                                    else {
                                        unlocked.session.close()
                                        reload()
                                    }
                                }
                            }
                        }
                    }
                    is E2eeStatus.Unlocked ->
                        SettingsSection(t("unlockedTitle"), t("unlockedDescription")) {}
                }
            is Phase.Created -> {
                val session = current.start.session
                SettingsSection(t("recoveryKeyTitle"), t("recoveryKeyDescription")) {
                    RecoveryKeyBox(current.start.recoveryKey)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(t("copy"), variant = "outline", icon = "copy", inlineStartIcon = true) {
                            host.copySecret(current.start.recoveryKey)
                            notice = t("copied")
                        }
                    }
                    Button(
                        t("recoveryKeyConfirm"),
                        variant = if (confirmed) "secondary" else "outline",
                        role = androidx.compose.ui.semantics.Role.Checkbox,
                        selected = confirmed,
                        icon = if (confirmed) "check" else null,
                        inlineStartIcon = true,
                        enabled = busy == null,
                    ) {
                        confirmed = !confirmed
                    }
                }
                SettingsSection(t("passkeyTitle"), t("passkeyDescription")) {
                    if (host.passkeys != null && host.rpId != null) {
                        Button(
                            if (passkeyAdded) t("passkeyAdded") else t("addPasskey"),
                            variant = "outline",
                            icon = if (passkeyAdded) "check" else "lock",
                            inlineStartIcon = true,
                            enabled = busy == null,
                        ) {
                            run(t("addPasskey")) {
                                host.setup.addSetupPasskey(session, host.passkeys, host.rpId, null)
                                passkeyAdded = true
                            }
                        }
                    } else {
                        Text(t("passkeyUnavailable"), muted = true, small = true)
                    }
                }
                SettingsSection("") {
                    Text(t("android.saveDescription"), muted = true, small = true)
                    Button(t("save"), primary = true, enabled = busy == null && confirmed) {
                        run(t("android.reauthWaiting")) {
                            // 再認証は、リカバリーキーを控えたあと、保存の直前に行う（同じ画面のまま）。
                            val reauth = host.reauthenticate()
                            busy = t("saving")
                            host.setup.completeSetup(session, reauth)
                            reload()
                        }
                    }
                }
            }
            is Phase.Continue ->
                SettingsSection(t("continueTitle"), t("android.continueDescription")) {
                    Button(t("continue"), primary = true, enabled = busy == null) {
                        run(t("android.reauthWaiting")) {
                            val reauth = host.reauthenticate()
                            busy = t("saving")
                            host.setup.saveRemaining(current.unlocked.session, current.unlocked.secretsRevision, reauth)
                            reload()
                        }
                    }
                }
        }
        busy?.let { label ->
            Progress(label)
            if (label == t("android.reauthWaiting")) {
                Button(t("android.reauthCancel"), variant = "outline") { job?.cancel() }
            }
        }
        if (notice.isNotEmpty()) Text(notice, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, muted = true, small = true)
        if (error.isNotEmpty()) {
            Text(
                error,
                Modifier.semantics { liveRegion = LiveRegionMode.Assertive },
                color = MaterialTheme.colorScheme.error,
                small = true,
            )
        }
    }
}

@Composable
private fun Progress(label: String) {
    androidx.compose.foundation.layout.Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        Text(label, muted = true, small = true)
    }
}

/** リカバリーキー（4 文字ごとに空白で区切った 48 文字）。等幅で表示し、選択できるようにする。 */
@Composable
private fun RecoveryKeyBox(recoveryKey: String) {
    SelectionContainer {
        androidx.compose.material3.Text(
            recoveryKey,
            Modifier.fillMaxWidth()
                .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
                .padding(12.dp),
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Medium,
            fontSize = 16.sp,
            lineHeight = 24.sp,
        )
    }
}
