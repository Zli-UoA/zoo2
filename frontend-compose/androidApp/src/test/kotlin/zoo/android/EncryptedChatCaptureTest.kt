package zoo.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import android.view.View
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import java.io.File
import kotlin.test.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import zoo.android.ui.AndroidScreen
import zoo.android.ui.AndroidScreens
import zoo.android.ui.InputState
import zoo.android.ui.TimelineItem

/**
 * 暗号化 DM の会話の画面（#361）。復号できない理由、送信者の印、暗号化の区切り、平文の警告、暗号化した添付、
 * 送信欄の上の表示を描き、明暗のキャプチャを `build/reports/ui` に残す（PR の確認用。画像の差分判定はしない）。
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h1000dp-xxhdpi", manifest = Config.NONE)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class EncryptedChatCaptureTest {
    private val screen = AndroidScreen.Conversation(
        "dm", "Bob", "",
        listOf(
            TimelineItem("p1", "Alice", "暗号化する前のメッセージ", "10:00"),
            TimelineItem("m1", "Alice", "こんにちは。資料を送ります", "10:01", divider = "ここから暗号化されています",
                attachments = listOf("a1" to "資料.pdf")),
            TimelineItem("m2", "Bob", "受け取りました", "10:02", marks = listOf("未検証の送信者")),
            TimelineItem("m3", "Bob", "", "10:03", notice = "このメッセージを復号する鍵がまだ届いていません。"),
            TimelineItem("m4", "Carol", "バックアップから読んだメッセージ", "10:04", marks = listOf("送信者を確かめられません")),
            TimelineItem("p2", "Mallory", "サーバーが足した平文", "10:05", marks = listOf("暗号化されていません")),
        ),
        InputState(notices = listOf(
            "前回の送信から増えた配布先：Carol",
            "次の人は新しいメッセージを読めません",
            "Carol さんは暗号化を設定していません",
        )),
    )

    private fun capture(dark: Boolean) {
        val controller = Robolectric.buildActivity(ComponentActivity::class.java)
        controller.get().setTheme(android.R.style.Theme_Material_Light_NoActionBar)
        val activity = controller.setup().get()
        activity.setContent { AndroidScreens(screen, dark = dark, foreground = true, dispatch = {}, photo = { _, _ -> }) }
        shadowOf(Looper.getMainLooper()).idle()
        val view = activity.window.decorView
        val width = activity.resources.displayMetrics.widthPixels
        val height = activity.resources.displayMetrics.heightPixels
        view.measure(View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY))
        view.layout(0, 0, width, height)
        shadowOf(Looper.getMainLooper()).idle()
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        val directory = File(System.getProperty("zoo.uiScreenshotDir") ?: "build/reports/ui").apply { mkdirs() }
        val file = File(directory, "e2ee-dm-${if (dark) "dark" else "light"}.png")
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        assertTrue(file.length() > 0)
    }

    @Test fun light() = capture(dark = false)
    @Test fun dark() = capture(dark = true)
}
