package zoo.android

import android.content.ClipData
import android.content.Context
import android.widget.Toast
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import androidx.activity.compose.BackHandler
import kotlinx.coroutines.Job
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import zoo.android.ui.*

@Composable
internal fun SignInRoute(
    dark: Boolean,
    restoring: Boolean,
    error: String,
    retry: (() -> Unit)?,
    cancelRestore: (() -> Unit)?,
    restorationStage: LoginStage = LoginStage.SavedSession,
    restoreDiagnostics: String,
    connect: suspend (LoginConnection, (LoginStage) -> Unit) -> Unit,
) {
    if (restoring) {
        RestorationPage(dark, restorationStage.label, cancelRestore)
        return
    }
    val context = LocalContext.current
    val preferences = remember(context) { context.getSharedPreferences("connection", Context.MODE_PRIVATE) }
    var server by rememberSaveable { mutableStateOf(preferences.getString("base", "https://zli.chat").orEmpty()) }
    var issuer by rememberSaveable { mutableStateOf(preferences.getString("issuer", "https://account.zli.works/api/auth").orEmpty()) }
    var client by rememberSaveable { mutableStateOf(preferences.getString("client", "").orEmpty()) }
    var resource by rememberSaveable { mutableStateOf(preferences.getString("resource", "https://zli.chat/api").orEmpty()) }
    var editing by rememberSaveable { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var signInError by rememberSaveable { mutableStateOf("") }
    var signInDiagnostics by rememberSaveable { mutableStateOf("") }
    var showDiagnostics by rememberSaveable { mutableStateOf(false) }
    val diagnostics = signInDiagnostics.ifBlank { restoreDiagnostics }
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    var loginJob by remember { mutableStateOf<Job?>(null) }
    BackHandler(editing && !busy && !restoring) { editing = false }
    val submit = {
        if (!busy) {
            val allowPortalAttempt = signInError == wifiSignInMessage
            busy = true
            loginJob = scope.launch {
                signInError = ""
                signInDiagnostics = ""
                val stages = mutableListOf(LoginStage.Network)
                try {
                    if (wifiNeedsSignIn(context) && !allowPortalAttempt) {
                        throw LoginFailure(wifiSignInMessage)
                    }
                    stages.add(LoginStage.Configuration)
                    val connection = LoginConnection(server.trim().trimEnd('/'), issuer.trim(), client.trim(), resource.trim())
                    connection.validate()
                    preferences.edit().putString("base", connection.base).putString("issuer", connection.issuer)
                        .putString("client", connection.client).putString("resource", connection.resource).apply()
                    connect(connection) { stages.add(it) }
                } catch (failure: CancellationException) {
                    throw failure
                } catch (failure: Exception) {
                    signInDiagnostics = loginDiagnosticLog(stages, failure)
                    signInError = when (failure) {
                        is zoo.crypto.ReauthException, is LoginFailure, is IllegalArgumentException -> failure.message.orEmpty()
                        else -> loginNetworkMessage(failure, wifiNeedsSignIn(context))
                            ?: "ログインできません。接続先とネットワークを確認してください。"
                    }
                } finally {
                    busy = false
                    loginJob = null
                }
            }
        }
    }
    AndroidShell(null, dark, {}) { colors ->
        if (editing && !busy) {
            PageHeader("接続先の設定", colors, onBack = { editing = false })
            Column(Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)) {
                LabeledField("Zoo サーバー URL", server, colors) { server = it }
                LabeledField("OIDC issuer", issuer, colors) { issuer = it }
                LabeledField("Android 用 public client ID", client, colors) { client = it }
                LabeledField("Resource / audience", resource, colors) { resource = it }
                (signInError.ifBlank { error }).takeIf(String::isNotEmpty)?.let { Text(it, color = colors.foreground) }
                if (diagnostics.isNotBlank()) TextAction("診断ログを見る", colors, icon = UiIcon.Document,
                    style = TextActionStyle.Plain) { showDiagnostics = true }
                TextAction(if (signInError == wifiSignInMessage) "それでも試す" else "ログイン", colors, icon = UiIcon.SignIn, enabled = client.isNotBlank(), click = submit)
            }
        } else {
            val pending = busy
            val description = if (pending) "" else signInError.ifBlank { error }.ifBlank {
                "ブラウザーで認証を完了すると、Zooに戻ります。"
            }
            BrandPage("Zoo", description, colors, loading = pending) {
                if (busy) {
                    TextAction("キャンセル", colors, icon = UiIcon.Close, style = TextActionStyle.Plain) { loginJob?.cancel() }
                } else {
                    if (diagnostics.isNotBlank()) TextAction("診断ログを見る", colors, icon = UiIcon.Document,
                        style = TextActionStyle.Plain) { showDiagnostics = true }
                    retry?.let { TextAction("自動ログインを再試行", colors, icon = UiIcon.Refresh, style = TextActionStyle.Plain) {
                        signInError = ""
                        signInDiagnostics = ""
                        showDiagnostics = false
                        it()
                    } }
                    if (client.isNotBlank()) {
                        TextAction("接続先の設定", colors, icon = UiIcon.Settings, style = TextActionStyle.Plain) { editing = true }
                    }
                    TextAction(if (client.isBlank()) "接続先を設定" else if (signInError == wifiSignInMessage) "それでも試す" else "ログイン", colors,
                        icon = if (client.isBlank()) UiIcon.Settings else UiIcon.SignIn) {
                        if (client.isBlank()) editing = true else submit()
                    }
                }
            }
        }

        if (showDiagnostics && diagnostics.isNotBlank()) {
            AlertDialog(
                onDismissRequest = { showDiagnostics = false },
                title = { Text("ログインの診断ログ") },
                text = {
                    SelectionContainer {
                        Text(diagnostics, Modifier.heightIn(max = 280.dp).verticalScroll(rememberScrollState()))
                    }
                },
                confirmButton = {
                    DialogAction("コピー", UiIcon.Copy) {
                        scope.launch {
                            clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("Zoo", diagnostics)))
                            if (android.os.Build.VERSION.SDK_INT < 33) {
                                Toast.makeText(context, "診断ログをコピーしました", Toast.LENGTH_SHORT).show()
                            }
                        }
                    }
                },
                dismissButton = { DialogAction("閉じる", UiIcon.Close) { showDiagnostics = false } },
            )
        }
    }
}

private fun wifiNeedsSignIn(context: Context): Boolean {
    val manager = context.getSystemService(ConnectivityManager::class.java) ?: return false
    val network = manager.activeNetwork ?: return false
    return manager.getNetworkCapabilities(network)
        ?.hasCapability(NetworkCapabilities.NET_CAPABILITY_CAPTIVE_PORTAL) == true
}
