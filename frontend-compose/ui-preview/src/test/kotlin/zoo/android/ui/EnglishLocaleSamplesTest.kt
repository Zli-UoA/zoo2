package zoo.android.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** 英語の辞書で描くと、ナビゲーション・入力欄・ボタンなどの固定の文言が英語になること。 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "en-rUS-w360dp-h760dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class EnglishLocaleSamplesTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private val english = generateSequence(File("").absoluteFile) { it.parentFile }
        .first { File(it, "frontend/src/lib/i18n/locales/en.json").isFile }
        .resolve("frontend/src/lib/i18n/locales/en.json").readText().let { Json.parseToJsonElement(it).jsonObject }

    /** 辞書を引く（アプリの translateDictionary と同じ。count が 1 なら _one、ほかは _other）。 */
    private fun lookup(key: String, args: Array<out Pair<String, String>>): String {
        fun find(path: String): String? {
            var value: JsonElement? = english
            path.split('.').forEach { value = (value as? JsonObject)?.get(it) }
            return (value as? JsonPrimitive)?.content
        }
        val count = args.toMap()["count"]?.toDoubleOrNull()
        var text = (if (count != null) find(key + if (count == 1.0) "_one" else "_other") else null) ?: find(key) ?: key
        args.forEach { (name, value) -> text = text.replace("{{$name}}", value) }
        return text
    }

    private fun show(screen: AndroidScreen) = ui.setContent {
        CompositionLocalProvider(LocalAndroidTranslation provides { key, args -> lookup(key, args) }) {
            AndroidScreens(screen, false, true, dispatch = {}, language = "en") { _, _ -> }
        }
    }

    @Test fun channelListIsEnglish() {
        show(AndroidScreen.Channels(listOf(
            ChannelSection("Pinned", listOf(ChannelItem("ops", "ops"))),
            ChannelSection("Joined", listOf(ChannelItem("dev", "dev", threads = listOf(ChannelThreadItem("t1", "Release checklist", 12))),
                ChannelItem("random", "random", unread = true))),
            ChannelSection("Direct messages", listOf(ChannelItem("dm", "mizuki", isUserName = true)), dm = true),
        ), moreThreads = true))
        ui.onAllNodesWithText("Channels").assertCountEquals(2) // 見出しと下のタブ
        ui.onAllNodesWithText("チャンネル", substring = true).assertCountEquals(0)
        capture("en-channels")
    }

    @Test fun conversationIsEnglish() {
        show(AndroidScreen.Conversation("dev", "dev", "", listOf(
            TimelineItem("m1", "mizuki", "I put the release notes draft here.", "11:02", dateDivider = "Today"),
        ), InputState()))
        ui.onAllNodesWithText("メッセージ", substring = true).assertCountEquals(0)
        capture("en-conversation")
    }

    @Test fun profileFollowButtonIsEnglish() {
        show(AndroidScreen.Profile("haru", "Testing the Android app.", emptyList(), FollowState.Unfollow, handle = "@haru@social.example"))
        ui.onNodeWithText("Unfollow").assertExists()
        capture("en-profile")
    }

    private fun capture(name: String) {
        ui.runOnIdle {
            val view = ui.activity.findViewById<View>(android.R.id.content)
            val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
            try {
                view.draw(Canvas(bitmap))
                val out = File("build/reports/android-ui").apply { mkdirs() }
                out.resolve("$name.png").outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            } finally { bitmap.recycle() }
        }
    }
}
