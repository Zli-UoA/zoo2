package zoo.android.update

import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.net.Uri
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import rikka.shizuku.Shizuku
import works.zli.zoo.BuildConfig

/** Native-only settings remain outside the shared Web/Android application screens. */
@Composable
internal fun AndroidUpdateFrame(context: Context, dark: Boolean, content: @Composable () -> Unit) {
    var show by remember { mutableStateOf(false) }
    MaterialTheme(colorScheme = if (dark) darkColorScheme() else lightColorScheme()) {
        Column(Modifier.fillMaxSize().safeDrawingPadding()) {
            Surface(tonalElevation = 1.dp) {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 12.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        "Android ${BuildConfig.VERSION_NAME}",
                        Modifier.padding(vertical = 14.dp),
                        style = MaterialTheme.typography.labelMedium,
                    )
                    TextButton(onClick = { show = true }) { Text("アプリの更新") }
                }
            }
            Box(Modifier.weight(1f)) { content() }
        }
        if (show) UpdateSettingsDialog(context) { show = false }
    }
}

@Composable
private fun UpdateSettingsDialog(context: Context, close: () -> Unit) {
    val prefs = remember { UpdatePreferences(context) }
    var enabled by remember { mutableStateOf(prefs.enabled) }
    var status by remember { mutableStateOf(prefs.status) }
    var ready by remember { mutableStateOf(shizukuReady()) }
    var granting by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
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
    AlertDialog(
        onDismissRequest = close,
        title = { Text("アプリの更新") },
        text = {
            Column(
                Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    "main の最新ビルドに更新します。自動更新は6時間ごとに、従量制ではないネットワークと十分なバッテリーがあるときに確認します。更新時はアプリが終了することがあります。"
                )
                Text(if (ready) "Shizuku: 使用を許可済み" else "Shizuku: 起動と使用許可が必要です")
                if (BuildConfig.DEBUG) Text("自動更新は署名付き release 版で利用できます。debug 版からの上書き更新はできません。")
                if (!ready) {
                    Button(
                        onClick = {
                            error = ""
                            try {
                                check(Shizuku.pingBinder() && !Shizuku.isPreV11()) {
                                    "Shizuku を起動してください"
                                }
                                granting = true
                                Shizuku.requestPermission(314)
                            } catch (_: Exception) {
                                granting = false
                                error = "Shizuku アプリを起動し、サービスを開始してから許可してください"
                            }
                        },
                        enabled = !granting && !BuildConfig.DEBUG,
                    ) {
                        Text("Shizuku の使用を許可")
                    }
                    TextButton(
                        onClick = {
                            context.startActivity(
                                Intent(
                                    Intent.ACTION_VIEW,
                                    Uri.parse("https://shizuku.rikka.app/ja/"),
                                )
                            )
                        }
                    ) {
                        Text("Shizuku の導入方法")
                    }
                }
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    Text("自動更新", Modifier.padding(top = 12.dp))
                    Switch(
                        checked = enabled,
                        onCheckedChange = {
                            UpdateScheduler.configure(context, it)
                            if (it) UpdateScheduler.checkNow(context)
                        },
                        enabled = !BuildConfig.DEBUG && (ready || enabled),
                    )
                }
                Button(
                    onClick = { UpdateScheduler.checkNow(context) },
                    enabled = ready && !BuildConfig.DEBUG,
                ) {
                    Text("今すぐ更新を確認・適用")
                }
                Text(status, style = MaterialTheme.typography.bodySmall)
                if (error.isNotBlank()) Text(error, color = MaterialTheme.colorScheme.error)
            }
        },
        confirmButton = { TextButton(onClick = close) { Text("閉じる") } },
    )
}
