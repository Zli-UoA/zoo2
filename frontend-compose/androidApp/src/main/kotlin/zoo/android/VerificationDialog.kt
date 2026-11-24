package zoo.android

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import zoo.Session
import zoo.android.ui.*
import zoo.crypto.QrScanResult
import zoo.crypto.SasController
import zoo.crypto.SasEntry
import zoo.crypto.SasErrorCode
import zoo.crypto.SasSaveState
import zoo.crypto.ffi.SasPhase
import zoo.crypto.ffi.VerificationMethod
import zoo.sasEmojiItems
import zoo.text
import zoo.author

/** 中止の理由の文（Web の `e2ee.sas.cancelled.*`）。 */
internal fun cancelledKey(code: String, byUs: Boolean): String = when (code) {
    "m.user" -> if (byUs) "e2ee.sas.cancelled.user" else "e2ee.sas.cancelled.userByThem"
    "m.timeout" -> "e2ee.sas.cancelled.timeout"
    "m.mismatched_sas" -> "e2ee.sas.cancelled.mismatchedSas"
    "m.key_mismatch" -> "e2ee.sas.cancelled.keyMismatch"
    "m.accepted" -> "e2ee.sas.cancelled.accepted"
    "m.unknown_method" -> "e2ee.sas.cancelled.unknownMethod"
    else -> "e2ee.sas.cancelled.other"
}

/**
 * 他のユーザーの検証（絵文字 SAS・QR。docs/e2ee.md の「絵文字 SAS 検証」「QR コードによる検証」）の画面。
 * 届いた要求も、自分から始めた検証も、アプリのどの画面にいても前面に出す（Web の `SasDialog`）。
 * 終わった検証は、閉じるまで結果を出す。
 */
@Composable
internal fun VerificationHost(session: Session, platform: AndroidPlatform) {
    val controller = platform.e2ee?.sas ?: return
    val entries by controller.entries.collectAsState()
    val entry = entries.firstOrNull() ?: return
    val scope = rememberCoroutineScope()
    val users by session.repo.users.rows.collectAsState()
    val name = users.firstOrNull { it.text("id") == entry.view.theirUserId }?.let(::author)?.ifBlank { null }
        ?: entry.view.theirUserId
    AndroidShellColors(platform) { colors ->
        Dialog(
            onDismissRequest = {},
            properties = DialogProperties(dismissOnBackPress = false, dismissOnClickOutside = false, usePlatformDefaultWidth = false),
        ) {
            VerificationContent(entry, name, colors, session::t, controller, scope, platform)
        }
    }
}

@Composable
private fun AndroidShellColors(platform: AndroidPlatform, content: @Composable (AndroidColors) -> Unit) {
    content(remember(platform.env.dark) { AndroidColors(platform.env.dark) })
}

@Composable
internal fun VerificationContent(
    entry: SasEntry,
    name: String,
    colors: AndroidColors,
    t: (String, Array<out Pair<String, String>>) -> String,
    controller: SasController?,
    scope: CoroutineScope,
    platform: AndroidPlatform?,
) {
    val view = entry.view
    val txn = view.transactionId
    fun tr(key: String) = t(key, arrayOf("name" to name))
    fun run(block: suspend SasController.() -> Unit) {
        val target = controller ?: return
        scope.launch {
            try { target.block() } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { }
        }
    }
    /** 押したときに再認証を始める（Web と同じく、押した操作の中でログインの画面を開く）。 */
    // 再認証の取り消しや時間切れは値（null）にする。例外のまま返すと、親のスコープまで取り消され、ダイアログの
    // ボタンが効かなくなるため。
    fun reauth() = scope.async {
        try { platform?.reauthenticate?.invoke() } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { null }
    }
    var scanning by remember(txn) { mutableStateOf(false) }
    var camera by remember(txn) { mutableStateOf<CameraState?>(null) }
    var otherCode by remember(txn) { mutableStateOf(false) }
    val enabled = !entry.busy
    Column(
        Modifier.fillMaxWidth().padding(16.dp).background(colors.background).padding(20.dp)
            .verticalScroll(rememberScrollState()).testTag("verification-$txn"),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        @Composable fun title(key: String) = Text(tr(key), color = colors.foreground, fontSize = 18.sp, fontWeight = AndroidWeights.emphasis)
        @Composable fun body(text: String) = Text(text, color = colors.secondary, fontSize = 14.sp)
        @Composable fun cancel() = TextAction(tr("e2ee.sas.cancel"), colors, enabled, TextActionStyle.Neutral, icon = UiIcon.Close) {
            run { cancel(txn) }
        }
        when (view.phase) {
            SasPhase.INCOMING -> {
                title("e2ee.sas.incomingTitle")
                body(tr("e2ee.sas.incomingDescription"))
                TextAction(tr("e2ee.sas.accept"), colors, enabled, icon = UiIcon.Check) { run { accept(txn) } }
                TextAction(tr("e2ee.sas.decline"), colors, enabled, TextActionStyle.Neutral, icon = UiIcon.Close) { run { cancel(txn) } }
            }
            SasPhase.REQUESTED -> {
                title("e2ee.sas.requestedTitle")
                body(tr("e2ee.sas.requestedDescription"))
                view.qrCode?.let { QrImage(it, tr("e2ee.qr.codeLabel")); body(tr("e2ee.qr.waitingForAccept")) }
                cancel()
            }
            SasPhase.CHOOSE_METHOD -> {
                title("e2ee.qr.title")
                val qr = view.qrCode
                if (scanning) {
                    body(tr("e2ee.qr.scanHint"))
                    if (otherCode) Text(tr("e2ee.qr.otherCode"), color = colors.foreground, fontSize = 14.sp)
                    when (camera) {
                        CameraState.DENIED -> body(tr("e2ee.qr.camera.denied"))
                        CameraState.NO_CAMERA -> body(tr("e2ee.qr.camera.noCamera"))
                        CameraState.FAILED -> body(tr("e2ee.qr.camera.failed"))
                        CameraState.STARTING, null -> body(tr("e2ee.qr.camera.starting"))
                        CameraState.RUNNING -> Unit
                    }
                    if (camera != CameraState.DENIED && camera != CameraState.NO_CAMERA && camera != CameraState.FAILED) {
                        QrScanner(tr("e2ee.qr.camera.label"), onCode = { bytes ->
                            val result = controller?.scanQr(txn, bytes)
                            otherCode = result == QrScanResult.RETRY
                            result == QrScanResult.ACCEPTED
                        }, onState = { camera = it })
                    }
                    if (qr != null) TextAction(tr("e2ee.qr.showOwn"), colors, enabled, TextActionStyle.Neutral) { scanning = false }
                } else {
                    // Web と同じく、出せるときは自分の QR を先に出し、相手の QR を読み取る操作を添える。
                    if (qr != null) {
                        QrImage(qr, tr("e2ee.qr.codeLabel"))
                        body(tr("e2ee.qr.showHint"))
                    }
                    if (view.canScanQrCode) TextAction(tr("e2ee.qr.scan"), colors, enabled,
                        if (qr != null) TextActionStyle.Neutral else TextActionStyle.Primary, icon = UiIcon.Photo) { scanning = true }
                }
                if (view.canStartSas) TextAction(tr("e2ee.qr.useEmoji"), colors, enabled, TextActionStyle.Neutral, icon = UiIcon.Emoji) {
                    run { startSas(txn) }
                }
                cancel()
            }
            SasPhase.QR_SCANNED -> {
                title("e2ee.qr.scannedTitle")
                body(tr("e2ee.qr.waitingForConfirm"))
                cancel()
            }
            SasPhase.QR_CONFIRM -> {
                title("e2ee.qr.confirmTitle")
                body(tr("e2ee.qr.confirmDescription"))
                TextAction(tr("e2ee.qr.confirmYes"), colors, enabled, icon = UiIcon.Check) { val token = reauth(); run { confirmQr(txn, token) } }
                TextAction(tr("e2ee.qr.confirmNo"), colors, enabled, TextActionStyle.Neutral, icon = UiIcon.Close) { run { cancel(txn) } }
            }
            SasPhase.EXCHANGING -> {
                body(tr("e2ee.sas.exchanging"))
                cancel()
            }
            SasPhase.COMPARE -> {
                title("e2ee.sas.compareTitle")
                body(tr("e2ee.sas.compareDescription"))
                view.emoji?.let { EmojiRow(it, colors, tr("e2ee.sas.emojiList")) { key -> t(key, emptyArray()) } }
                TextAction(tr("e2ee.sas.match"), colors, enabled, icon = UiIcon.Check) { val token = reauth(); run { confirm(txn, token) } }
                TextAction(tr("e2ee.sas.mismatch"), colors, enabled, TextActionStyle.Neutral, icon = UiIcon.Close) { run { mismatch(txn) } }
            }
            SasPhase.CONFIRMED -> {
                body(tr("e2ee.sas.confirmed"))
                cancel()
            }
            SasPhase.DONE -> {
                title(if (view.method == VerificationMethod.QR_CODE) "e2ee.qr.verifiedTitle" else "e2ee.sas.savedTitle")
                when (entry.save) {
                    SasSaveState.SAVING -> body(tr("e2ee.sas.saving"))
                    SasSaveState.SAVED -> body(tr("e2ee.sas.savedDescription"))
                    SasSaveState.LOGIN -> {
                        body(tr("e2ee.qr.needsLogin"))
                        TextAction(tr("e2ee.qr.login"), colors, enabled, icon = UiIcon.SignIn) { val token = reauth(); run { retrySave(txn, token) } }
                    }
                    SasSaveState.REAUTH, SasSaveState.FAILED -> {
                        body(tr(if (entry.save == SasSaveState.REAUTH) "e2ee.sas.reauth" else "e2ee.sas.saveFailed"))
                        TextAction(tr("e2ee.sas.retrySave"), colors, enabled, icon = UiIcon.Refresh) { val token = reauth(); run { retrySave(txn, token) } }
                    }
                    SasSaveState.IDLE -> Unit
                }
                if (entry.save != SasSaveState.SAVING) {
                    TextAction(tr("e2ee.sas.close"), colors, enabled, TextActionStyle.Neutral) { run { dismiss(txn) } }
                }
            }
            SasPhase.CANCELLED -> {
                title("e2ee.sas.cancelledTitle")
                view.cancel?.let { body(tr(cancelledKey(it.code, it.byUs))) }
                TextAction(tr("e2ee.sas.close"), colors, true, TextActionStyle.Neutral) { run { dismiss(txn) } }
            }
        }
        entry.error?.let { error ->
            Text(tr(when (error) {
                SasErrorCode.NEEDS_UNLOCK -> "e2ee.sas.needsUnlock"
                SasErrorCode.SEND_FAILED -> "e2ee.sas.sendFailed"
                SasErrorCode.RECEIVE_FAILED -> "e2ee.sas.receiveFailed"
                SasErrorCode.FAILED -> "e2ee.sas.failed"
            }), color = colors.foreground, fontSize = 14.sp)
        }
    }
}

/** 検証の 7 つの絵文字と名前（Web の `SasEmojiList`）。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun EmojiRow(indices: ByteArray, colors: AndroidColors, label: String, translate: (String) -> String) {
    val items = sasEmojiItems(indices, translate)
    FlowRow(
        Modifier.fillMaxWidth().semantics(mergeDescendants = true) { contentDescription = label },
        horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterHorizontally),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items.forEach { item ->
            Column(Modifier.widthIn(min = 64.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(item.emoji, fontSize = 32.sp)
                Text(item.label, color = colors.secondary, fontSize = 12.sp)
            }
        }
    }
}

/** 検証の QR（白地に黒。周りに 4 モジュールの余白）。 */
@Composable
internal fun QrImage(bytes: ByteArray, label: String) {
    val modules = remember(bytes.contentHashCode()) { qrModules(bytes) }
    Canvas(Modifier.fillMaxWidth().aspectRatio(1f).background(Color.White).padding(16.dp)
        .semantics { contentDescription = label }) {
        val cell = size.minDimension / modules.size
        modules.forEachIndexed { y, row ->
            row.forEachIndexed { x, dark ->
                if (dark) drawRect(Color.Black, Offset(x * cell, y * cell), Size(cell + 0.5f, cell + 0.5f))
            }
        }
    }
}
