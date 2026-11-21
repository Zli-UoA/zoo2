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
import androidx.compose.ui.Alignment
import androidx.compose.ui.unit.dp
import rikka.shizuku.Shizuku
import works.zli.zoo.BuildConfig
import zoo.Button as ZooButton
import zoo.Text as ZooText

@Composable
internal fun UpdateSettingsContent(context: Context) {
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
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        ZooText("バージョン ${BuildConfig.VERSION_NAME}", muted = true, small = true)
        ZooText(
            "main の最新ビルドに更新します。自動更新は6時間ごとに、従量制ではないネットワークと十分なバッテリーがあるときに確認します。更新時はアプリが終了することがあります。"
        )
        ZooText(if (ready) "Shizuku: 使用を許可済み" else "Shizuku: 起動と使用許可が必要です")
        if (BuildConfig.DEBUG) ZooText("自動更新は署名付き release 版で利用できます。debug 版からの上書き更新はできません。")
        if (!ready) {
            ZooButton("Shizuku の使用を許可", enabled = !granting && !BuildConfig.DEBUG) {
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
            }
            ZooButton("Shizuku の導入方法") {
                context.startActivity(
                    Intent(Intent.ACTION_VIEW, Uri.parse("https://shizuku.rikka.app/ja/")),
                )
            }
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            ZooText("自動更新", Modifier.weight(1f))
            Switch(
                checked = enabled,
                onCheckedChange = {
                    UpdateScheduler.configure(context, it)
                    if (it) UpdateScheduler.checkNow(context)
                },
                enabled = !BuildConfig.DEBUG && (ready || enabled),
            )
        }
        ZooButton("今すぐ更新を確認・適用", enabled = ready && !BuildConfig.DEBUG, primary = true) {
            UpdateScheduler.checkNow(context)
        }
        if (status.isNotBlank()) ZooText(status, muted = true, small = true)
        if (error.isNotBlank()) ZooText(error, color = MaterialTheme.colorScheme.error)
    }
}
