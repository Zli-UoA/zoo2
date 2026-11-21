package zoo.android

import android.content.Context
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
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
    connect: suspend (LoginConnection) -> Unit,
) {
    val context = LocalContext.current
    val preferences = remember(context) { context.getSharedPreferences("connection", Context.MODE_PRIVATE) }
    var server by rememberSaveable { mutableStateOf(preferences.getString("base", "https://zli.chat").orEmpty()) }
    var issuer by rememberSaveable { mutableStateOf(preferences.getString("issuer", "https://account.zli.works/api/auth").orEmpty()) }
    var client by rememberSaveable { mutableStateOf(preferences.getString("client", "").orEmpty()) }
    var resource by rememberSaveable { mutableStateOf(preferences.getString("resource", "https://zli.chat/api").orEmpty()) }
    var editing by rememberSaveable { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var signInError by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val submit = {
        if (!busy) {
            busy = true
            scope.launch {
                signInError = ""
                try {
                    val connection = LoginConnection(server.trim().trimEnd('/'), issuer.trim(), client.trim(), resource.trim())
                    connection.validate()
                    preferences.edit().putString("base", connection.base).putString("issuer", connection.issuer)
                        .putString("client", connection.client).putString("resource", connection.resource).apply()
                    connect(connection)
                } catch (failure: CancellationException) {
                    throw failure
                } catch (failure: LoginFailure) {
                    signInError = failure.message.orEmpty()
                } catch (failure: IllegalArgumentException) {
                    signInError = failure.message.orEmpty()
                } catch (_: Exception) {
                    signInError = "ログインできません。接続先とネットワークを確認してください。"
                } finally {
                    busy = false
                }
            }
        }
    }
    AndroidShell(null, dark, {}) { colors ->
        if (editing && !restoring && !busy) {
            PageHeader("接続先の設定", colors, onBack = { editing = false })
            Column(Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(server, { server = it }, Modifier.fillMaxWidth(), label = { Text("Zoo サーバー URL") }, singleLine = true)
                OutlinedTextField(issuer, { issuer = it }, Modifier.fillMaxWidth(), label = { Text("OIDC issuer") }, singleLine = true)
                OutlinedTextField(client, { client = it }, Modifier.fillMaxWidth(), label = { Text("Android 用 public client ID") }, singleLine = true)
                OutlinedTextField(resource, { resource = it }, Modifier.fillMaxWidth(), label = { Text("Resource / audience") }, singleLine = true)
                (signInError.ifBlank { error }).takeIf(String::isNotEmpty)?.let { Text(it, color = colors.foreground) }
                TextAction("ログイン", colors, icon = UiIcon.SignIn, enabled = client.isNotBlank(), click = submit)
            }
        } else {
            val pending = restoring || busy
            val description = if (pending) "" else signInError.ifBlank { error }.ifBlank {
                "ブラウザーで認証を完了すると、Zooに戻ります。"
            }
            BrandPage("Zoo", description, colors, loading = pending) {
                if (restoring) {
                    TextAction("ログイン画面を開く", colors, icon = UiIcon.SignIn, enabled = cancelRestore != null, style = TextActionStyle.Plain) {
                        cancelRestore?.invoke()
                    }
                } else if (!busy) {
                    retry?.let { TextAction("自動ログインを再試行", colors, icon = UiIcon.Refresh, style = TextActionStyle.Plain, click = it) }
                    if (client.isNotBlank()) {
                        TextAction("接続先の設定", colors, icon = UiIcon.Settings, style = TextActionStyle.Plain) { editing = true }
                    }
                    TextAction(if (client.isBlank()) "接続先を設定" else "ログイン", colors,
                        icon = if (client.isBlank()) UiIcon.Settings else UiIcon.SignIn) {
                        if (client.isBlank()) editing = true else submit()
                    }
                }
            }
        }
    }
}
