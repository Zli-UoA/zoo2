package zoo.android

import android.app.*
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.SystemBarStyle
import zoo.android.ui.*
import android.content.*
import android.os.Bundle
import androidx.core.content.FileProvider
import kotlinx.serialization.json.JsonArray
import java.io.File
import zoo.obj

/** Debug-only fixture for capturing Android-owned surfaces with non-personal sample data. */
class SystemSurfaceSampleActivity : ComponentActivity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        if (intent.action == "zoo.sample.STATUS_BAR") {
            val dark = intent.getBooleanExtra("dark", false)
            val clear = android.graphics.Color.TRANSPARENT
            val style = if (dark) SystemBarStyle.dark(clear) else SystemBarStyle.light(clear, clear)
            enableEdgeToEdge(statusBarStyle = style, navigationBarStyle = style)
            val entries = (1..24).map { n ->
                TimelineItem("sample-$n", "はる", if (n == 1) "北口で受付を始めました。" else "展示の様子をお届けします。", "10:00",
                    canReact = true, canRepost = true, postHeader = PostHeaderData("@haru", "11/28 10:00", PostAudience.Public, null, "zli.chat"))
            }
            setContent {
                AndroidScreens(AndroidScreen.Home(entries, timeline = "home", showingSavedHead = true), dark, true,
                    dispatch = {}, language = "ja") { _, _ -> }
            }
            return
        }
        if (intent.action == "zoo.sample.SHARE") {
            val file = File(cacheDir, "preview-files/展示メモ.txt")
            file.parentFile?.mkdirs()
            file.writeText("明日の展示は10時からです。")
            val uri = FileProvider.getUriForFile(this, "${packageName}.preview-files", file)
            startActivity(Intent.createChooser(fileShareIntent(contentResolver, uri, file.name, "text/plain"), file.name))
        } else {
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(NotificationChannel("sample", "画面検証", NotificationManager.IMPORTANCE_DEFAULT))
            listOf("mention", "reaction").forEachIndexed { index, kind ->
                val row = obj("actor_id" to "haru", "notification_type" to kind,
                    "display_data" to obj("actor" to obj("display_name" to "はる"), "channel" to obj("channel_name" to "開発"),
                        "message" to obj("content" to "明日の展示は10時からです。"),
                        "reactions" to JsonArray(listOf(obj("user_id" to "haru", "emoji" to "👏")))))
                val body = notificationBody(row) { _, actor -> "${actor}さんがリアクションしました" }
                manager.notify(7100 + index, Notification.Builder(this, "sample")
                    .setSmallIcon(works.zli.zoo.R.drawable.ic_notification).setContentTitle("はる · 開発")
                    .setContentText(body).setStyle(notificationStyle(row, body, "自分", null)).build())
            }
        }
        finish()
    }
}
