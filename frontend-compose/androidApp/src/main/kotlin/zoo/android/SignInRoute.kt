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
import androidx.compose.material3.OutlinedTextField
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
    restoreDiagnostics: String,
    connect: suspend (LoginConnection, (LoginStage) -> Unit) -> Unit,
) {
    val context = LocalContext.current
    val preferences = remember(context) { context.getSharedPreferences("connection", Context.MODE_PRIVATE) }
    var server by rememberSaveable { mutableStateOf(preferences.getString("base", "https://zli.chat").orEmpty()) }
    var issuer by rememberSaveable { mutableStateOf(preferences.getString("issuer", "https://account.zli.works/api/auth").orEmpty()) }
    var client by rememberSaveable { mutableStateOf(preferences.getString("client", "").orEmpty()) }
    var resource by rememberSaveable { mutableStateOf(preferences.getString("resource", "https://zli.chat/api").orEmpty()) }
    var editing by rememberSaveable { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var signInError by rememberSaveable { mutableStateOf("") }
    // Wi-Fi のログイン（キャプティブポータル）が要る、と出しているとき（「それでも試す」を出す）。
    var signInPortal by rememberSaveable { mutableStateOf(false) }
    var signInDiagnostics by rememberSaveable { mutableStateOf("") }
    var showDiagnostics by rememberSaveable { mutableStateOf(false) }
    val diagnostics = signInDiagnostics.ifBlank { restoreDiagnostics }
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    var loginJob by remember { mutableStateOf<Job?>(null) }
    BackHandler(editing && !busy && !restoring) { editing = false }
    val submit = {
        if (!busy) {
            val allowPortalAttempt = signInPortal
            busy = true
            loginJob = scope.launch {
                signInError = ""
                signInPortal = false
                signInDiagnostics = ""
                val stages = mutableListOf(LoginStage.Network)
                try {
                    if (wifiNeedsSignIn(context) && !allowPortalAttempt) {
                        throw LoginFailure(wifiSignInMessage) { it.t("android.login.wifiSignIn") }
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
                    val text = context.loginText()
                    val shown = loginErrorText(failure, text)
                    if (shown != null) {
                        signInError = shown
                        signInPortal = failure is LoginFailure && failure.message == wifiSignInMessage
                    } else {
                        val captivePortal = wifiNeedsSignIn(context)
                        signInError = loginNetworkText(failure, captivePortal, text)
                            ?: text.t("android.login.failedGeneric")
                        signInPortal = captivePortal
                    }
                } finally {
                    busy = false
                    loginJob = null
                }
            }
        }
    }
    AndroidShell(null, dark, {}, allowRail = false) { colors ->
        if (editing && !restoring && !busy) {
            PageHeader(androidLabel("android.signIn.connectionSettings", "接続先の設定"), colors, onBack = { editing = false })
            Column(Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(server, { server = it }, Modifier.fillMaxWidth(), label = { Text(androidLabel("android.signIn.serverUrl", "Zoo サーバー URL")) }, singleLine = true)
                OutlinedTextField(issuer, { issuer = it }, Modifier.fillMaxWidth(), label = { Text("OIDC issuer") }, singleLine = true)
                OutlinedTextField(client, { client = it }, Modifier.fillMaxWidth(), label = { Text(androidLabel("android.signIn.clientId", "Android 用 public client ID")) }, singleLine = true)
                OutlinedTextField(resource, { resource = it }, Modifier.fillMaxWidth(), label = { Text("Resource / audience") }, singleLine = true)
                (signInError.ifBlank { error }).takeIf(String::isNotEmpty)?.let { Text(it, color = colors.foreground) }
                if (diagnostics.isNotBlank()) TextAction(androidLabel("android.signIn.viewDiagnostics", "診断ログを見る"), colors, icon = UiIcon.Document,
                    style = TextActionStyle.Plain) { showDiagnostics = true }
                TextAction(if (signInPortal) androidLabel("android.signIn.tryAnyway", "それでも試す") else androidLabel("auth.login", "ログイン"), colors, icon = UiIcon.SignIn, enabled = client.isNotBlank(), click = submit)
            }
        } else {
            val pending = restoring || busy
            val browserHint = androidLabel("android.signIn.browserHint", "ブラウザーで認証を完了すると、Zooに戻ります。")
            val description = if (pending) "" else signInError.ifBlank { error }.ifBlank { browserHint }
            BrandPage("Zoo", description, colors, loading = pending) {
                if (restoring) {
                    TextAction(androidLabel("android.signIn.openLogin", "ログイン画面を開く"), colors, icon = UiIcon.SignIn, enabled = cancelRestore != null, style = TextActionStyle.Plain) {
                        cancelRestore?.invoke()
                    }
                } else if (busy) {
                    TextAction(androidLabel("common.cancel", "キャンセル"), colors, icon = UiIcon.Close, style = TextActionStyle.Plain) { loginJob?.cancel() }
                } else {
                    if (diagnostics.isNotBlank()) TextAction(androidLabel("android.signIn.viewDiagnostics", "診断ログを見る"), colors, icon = UiIcon.Document,
                        style = TextActionStyle.Plain) { showDiagnostics = true }
                    retry?.let { TextAction(androidLabel("android.signIn.retryAutoLogin", "自動ログインを再試行"), colors, icon = UiIcon.Refresh, style = TextActionStyle.Plain) {
                        signInError = ""
                        signInPortal = false
                        signInDiagnostics = ""
                        showDiagnostics = false
                        it()
                    } }
                    if (client.isNotBlank()) {
                        TextAction(androidLabel("android.signIn.connectionSettings", "接続先の設定"), colors, icon = UiIcon.Settings, style = TextActionStyle.Plain) { editing = true }
                    }
                    TextAction(
                        if (client.isBlank()) androidLabel("android.signIn.setUpConnection", "接続先を設定")
                        else if (signInPortal) androidLabel("android.signIn.tryAnyway", "それでも試す")
                        else androidLabel("auth.login", "ログイン"),
                        colors,
                        icon = if (client.isBlank()) UiIcon.Settings else UiIcon.SignIn) {
                        if (client.isBlank()) editing = true else submit()
                    }
                }
            }
        }

        if (showDiagnostics && diagnostics.isNotBlank()) {
            AlertDialog(
                onDismissRequest = { showDiagnostics = false },
                title = { Text(androidLabel("android.signIn.diagnosticsTitle", "ログインの診断ログ")) },
                text = {
                    SelectionContainer {
                        Text(diagnostics, Modifier.heightIn(max = 280.dp).verticalScroll(rememberScrollState()))
                    }
                },
                confirmButton = {
                    DialogAction(androidLabel("e2ee.copy", "コピー"), UiIcon.Copy) {
                        scope.launch {
                            clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("Zoo", diagnostics)))
                            if (android.os.Build.VERSION.SDK_INT < 33) {
                                Toast.makeText(context, context.appText("android.signIn.diagnosticsCopied"), Toast.LENGTH_SHORT).show()
                            }
                        }
                    }
                },
                dismissButton = { DialogAction(androidLabel("common.close", "閉じる"), UiIcon.Close) { showDiagnostics = false } },
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
