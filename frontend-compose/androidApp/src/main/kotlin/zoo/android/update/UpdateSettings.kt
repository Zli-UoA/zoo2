package zoo.android.update

import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.net.Uri
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.Alignment
import androidx.compose.ui.unit.dp
import io.ktor.client.*
import io.ktor.client.engine.okhttp.*
import io.ktor.client.plugins.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import rikka.shizuku.Shizuku
import works.zli.zoo.BuildConfig
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.unit.sp
import zoo.android.ui.AndroidShell
import zoo.android.ui.PageHeader
import zoo.android.ui.TextAction
import zoo.android.ui.TextActionStyle
import zoo.android.ui.UiIcon

@Composable
internal fun UpdateSettings(context: Context, dark: Boolean, language: String, back: () -> Unit) {
    val prefs = remember { UpdatePreferences(context) }
    var enabled by remember { mutableStateOf(prefs.enabled) }
    var status by remember { mutableStateOf(prefs.status) }
    var ready by remember { mutableStateOf(shizukuReady()) }
    var granting by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var fetching by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    DisposableEffect(Unit) {
        val changed =
            SharedPreferences.OnSharedPreferenceChangeListener { _, _ ->
                enabled = prefs.enabled
                status = prefs.status
            }
        val received = Shizuku.OnBinderReceivedListener { ready = shizukuReady() }
        val dead =
            Shizuku.OnBinderDeadListener {
                ready = false
                granting = false
            }
        val permission =
            Shizuku.OnRequestPermissionResultListener { request, grant ->
                if (request == 314) {
                    granting = false
                    ready = grant == PackageManager.PERMISSION_GRANTED && shizukuReady()
                    if (!ready) error = "Shizuku の使用が許可されませんでした"
                }
            }
        prefs.values.registerOnSharedPreferenceChangeListener(changed)
        Shizuku.addBinderReceivedListenerSticky(received)
        Shizuku.addBinderDeadListener(dead)
        Shizuku.addRequestPermissionResultListener(permission)
        onDispose {
            prefs.values.unregisterOnSharedPreferenceChangeListener(changed)
            Shizuku.removeBinderReceivedListener(received)
            Shizuku.removeBinderDeadListener(dead)
            Shizuku.removeRequestPermissionResultListener(permission)
        }
    }
    AndroidShell(null, dark, {}, language) { colors ->
        PageHeader("アプリの更新", colors, onBack = back)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("バージョン ${BuildConfig.VERSION_NAME}", color = colors.secondary, fontSize = 13.sp)
            Text("自動更新は Wi-Fi などの従量制でない接続で確認します。インストールが終わると Zoo が開き直ります。",
                color = colors.foreground)
            Text(if (ready) "Shizuku の使用を許可済み" else "Shizuku の起動と使用許可が必要です",
                color = colors.secondary)
            if (BuildConfig.DEBUG) Text("自動更新は署名付きの配布版で利用できます。", color = colors.secondary)
            if (!ready && !BuildConfig.DEBUG) {
                TextAction("Shizuku の使用を許可", colors, icon = UiIcon.Key, enabled = !granting) {
                    error = ""
                    try {
                        check(Shizuku.pingBinder() && !Shizuku.isPreV11())
                        granting = true
                        Shizuku.requestPermission(314)
                    } catch (_: Exception) {
                        granting = false
                        error = "Shizuku のサービスを起動してから許可してください"
                    }
                }
            }
            if (!ready) TextAction("Shizuku の導入方法", colors, icon = UiIcon.External, style = TextActionStyle.Neutral) {
                try {
                    context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://shizuku.rikka.app/ja/")))
                } catch (_: android.content.ActivityNotFoundException) {
                    error = "ブラウザーを開けませんでした"
                }
            }
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text("自動更新", Modifier.weight(1f), color = colors.foreground)
                Switch(
                    checked = enabled,
                    onCheckedChange = {
                        UpdateScheduler.configure(context, it)
                    },
                    enabled = !BuildConfig.DEBUG && (ready || enabled),
                    modifier = Modifier.semantics { contentDescription = "自動更新" },
                )
            }
            TextAction("今すぐ更新を確認", colors, icon = UiIcon.Refresh, enabled = ready && !BuildConfig.DEBUG) {
                UpdateScheduler.checkNow(context)
            }
            TextAction("最新の APK をダウンロード", colors, icon = UiIcon.External, style = TextActionStyle.Neutral, enabled = !fetching) {
                error = ""
                fetching = true
                scope.launch {
                    try {
                        val release =
                            withContext(Dispatchers.IO) {
                                HttpClient(OkHttp) {
                                    engine { config { followSslRedirects(false) } }
                                    install(HttpTimeout) { requestTimeoutMillis = 30_000 }
                                }.use { fetchUpdateRelease(it) }
                            }
                        context.startActivity(
                            Intent(Intent.ACTION_VIEW, Uri.parse(release.url))
                                .addCategory(Intent.CATEGORY_BROWSABLE)
                        )
                    } catch (_: android.content.ActivityNotFoundException) {
                        error = "ブラウザーを開けませんでした"
                    } catch (e: kotlinx.coroutines.CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        error = e.message ?: "更新情報を取得できませんでした"
                    } finally {
                        fetching = false
                    }
                }
            }
            Text("Shizuku を使わない場合は、APK をダウンロードして開き、手動でインストールしてください。",
                color = colors.secondary, fontSize = 13.sp)
            if (status.isNotBlank()) Text(status, color = colors.secondary, fontSize = 13.sp)
            if (error.isNotBlank()) Text(error, color = MaterialTheme.colorScheme.error)
        }
    }
}
