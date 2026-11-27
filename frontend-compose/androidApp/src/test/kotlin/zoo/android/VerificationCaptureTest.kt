package zoo.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import android.view.View
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import java.io.File
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import zoo.android.ui.AndroidColors
import zoo.crypto.SasEntry
import zoo.crypto.ffi.SasPhase
import zoo.crypto.ffi.SasView
import zoo.crypto.ffi.VerificationMethod

/**
 * 他のユーザーの検証の画面（#362）。届いた要求、QR と方式の選択、絵文字の見比べ、検証できたときを描き、明暗の
 * キャプチャを `build/reports/ui` に残す（PR の確認用。画像の差分判定はしない）。
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h1100dp-xxhdpi", manifest = Config.NONE)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class VerificationCaptureTest {
    /** アプリと同じ Web の日本語の文言（`frontend/src/lib/i18n/locales/ja.json`）。 */
    private val labels: Map<String, String> = run {
        fun flatten(prefix: String, value: kotlinx.serialization.json.JsonElement): List<Pair<String, String>> = when (value) {
            is kotlinx.serialization.json.JsonObject -> value.flatMap { (key, child) -> flatten(if (prefix.isEmpty()) key else "$prefix.$key", child) }
            is kotlinx.serialization.json.JsonPrimitive -> listOf(prefix to value.content)
            else -> emptyList()
        }
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "frontend/src/lib/i18n/locales/ja.json") }.first { it.exists() }
        flatten("", kotlinx.serialization.json.Json.parseToJsonElement(file.readText())).toMap()
    }
    private val t: (String, Array<out Pair<String, String>>) -> String = { key, args ->
        args.fold(labels[key] ?: key) { text, (name, value) -> text.replace("{{$name}}", value) }
    }

    private fun view(phase: SasPhase, emoji: ByteArray? = null, qr: ByteArray? = null, method: VerificationMethod? = null) =
        SasView("txn-$phase", "bob", "DEVICE", false, 0uL, phase, emoji, null, null, method, qr, qr != null, true)

    private val qr = "MATRIX".toByteArray() + byteArrayOf(2, 0, 0, 4) + "txn1".toByteArray() + ByteArray(72) { (it * 37).toByte() }

    private fun capture(name: String, dark: Boolean, entry: SasEntry) {
        val controller = Robolectric.buildActivity(ComponentActivity::class.java)
        controller.get().setTheme(android.R.style.Theme_Material_Light_NoActionBar)
        val activity = controller.setup().get()
        val colors = AndroidColors(dark)
        activity.setContent {
            Column(Modifier.fillMaxSize().background(colors.quietSurface)) {
                VerificationContent(entry, "Bob", colors, t, null, rememberCoroutineScope(), null)
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        val root = activity.window.decorView
        val width = activity.resources.displayMetrics.widthPixels
        val height = activity.resources.displayMetrics.heightPixels
        root.measure(View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY))
        root.layout(0, 0, width, height)
        shadowOf(Looper.getMainLooper()).idle()
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        root.draw(Canvas(bitmap))
        val directory = File(System.getProperty("zoo.uiScreenshotDir") ?: "build/reports/ui").apply { mkdirs() }
        val file = File(directory, "e2ee-verify-$name-${if (dark) "dark" else "light"}.png")
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        assertTrue(file.length() > 0)
    }

    @Test fun incoming() = listOf(false, true).forEach { capture("incoming", it, SasEntry(view(SasPhase.INCOMING))) }
    @Test fun chooseMethodWithQr() = listOf(false, true).forEach { capture("qr", it, SasEntry(view(SasPhase.CHOOSE_METHOD, qr = qr))) }
    @Test fun compareEmoji() = listOf(false, true).forEach {
        capture("emoji", it, SasEntry(view(SasPhase.COMPARE, emoji = byteArrayOf(0, 7, 15, 22, 31, 40, 63))))
    }

    @Test fun cancelReasonsFollowWeb() {
        assertEquals("e2ee.sas.cancelled.userByThem", cancelledKey("m.user", byUs = false))
        assertEquals("e2ee.sas.cancelled.user", cancelledKey("m.user", byUs = true))
        assertEquals("e2ee.sas.cancelled.mismatchedSas", cancelledKey("m.mismatched_sas", byUs = true))
        assertEquals("e2ee.sas.cancelled.other", cancelledKey("m.something", byUs = false))
    }
}
