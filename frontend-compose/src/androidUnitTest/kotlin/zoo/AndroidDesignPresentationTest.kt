package zoo

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import kotlin.test.Test
import kotlin.test.assertEquals
import org.junit.Rule
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w480dp-h1000dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AndroidDesignPresentationTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun displayAndOperateTheSettingsDesignWithoutSessionOrNetwork() {
        var open: String? = null
        var swipe by mutableStateOf(true)
        var dark by mutableStateOf(false)
        ui.setContent {
            CompositionLocalProvider(LocalTouchUi provides true) {
                ZooTheme(dark = dark) {
                    Column(
                        Modifier.size(360.dp, 760.dp)
                            .background(MaterialTheme.colorScheme.background)
                            .testTag("unwired-settings"),
                    ) {
                        ScreenHeader("settings", "設定", brand = true)
                        Box(Modifier.padding(horizontal = AndroidDesign.screenInset)) {
                            AndroidSettingsIndex(
                                AndroidSettingsLabels(
                                    "外観", "テーマ", "端末に従う", "操作",
                                    "スワイプで返信", "触覚フィードバック", "アプリ",
                                ),
                                listOf("general" to "一般", "sip" to "通話", "members" to "メンバー"),
                                swipe, true, { open = it }, { swipe = !swipe }, {},
                            )
                        }
                    }
                }
            }
        }
        captureAndroidUi(ui, { ui.activity }, "unwired-settings", "android-ui-first-settings-light")
        ui.onNodeWithText("テーマ").performClick()
        ui.runOnIdle { assertEquals("appearance", open) }
        ui.onNodeWithContentDescription("スワイプで返信").assertIsOn().performClick().assertIsOff()
        ui.runOnIdle { dark = true }
        captureAndroidUi(ui, { ui.activity }, "unwired-settings", "android-ui-first-settings-dark")
    }
}
