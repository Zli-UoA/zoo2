package zoo.android.ui

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "ja-rJP-w360dp-h760dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ConversationScrollTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private val entries = (1..40).map { TimelineItem("e$it", "はる", "メッセージ $it", "12:00") }
    private val channel = AndroidScreen.Conversation("dev", "開発", "", entries, InputState())

    @Test
    fun inputFollowsTheHandleBeforeReleaseInBothDirections() {
        var screen by mutableStateOf(channel.copy(input = InputState("下書き")))
        ui.setContent {
            AndroidScreens(screen, false, true, dispatch = { action ->
                if (action == AndroidUiAction.UseInputTool(InputTool.Expand)) {
                    screen = screen.copy(input = screen.input.copy(expanded = !screen.input.expanded))
                }
            }) { _, _ -> }
        }

        val compact = ui.onNodeWithContentDescription("入力欄を広げる")
        val compactTop = compact.fetchSemanticsNode().boundsInRoot.top
        val delta = with(ui.density) { 120.dp.toPx() }
        compact.performTouchInput { down(center); moveBy(Offset(0f, -delta)) }
        assertTrue(compact.fetchSemanticsNode().boundsInRoot.top < compactTop - delta / 2)
        compact.performTouchInput { up() }

        val expanded = ui.onNodeWithContentDescription("入力欄を縮める")
        val expandedTop = expanded.fetchSemanticsNode().boundsInRoot.top
        expanded.performTouchInput { down(center); moveBy(Offset(0f, delta)) }
        assertTrue(expanded.fetchSemanticsNode().boundsInRoot.top > expandedTop + delta / 2)
        expanded.performTouchInput { cancel() }
        assertEquals(expandedTop, expanded.fetchSemanticsNode().boundsInRoot.top, 1f)
    }

    @Test
    fun viewportChangesFollowTheEndAndSuccessfulSendReturnsFromHistory() {
        var screen by mutableStateOf(channel)
        var height by mutableStateOf(750.dp)
        ui.setContent {
            Box(Modifier.height(height)) {
                AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> }
            }
        }

        val list = ui.onNodeWithTag("conversation-timeline")
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()
        ui.runOnIdle { height = 430.dp }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()

        list.performTouchInput { swipeDown() }
        list.performScrollToIndex(12)
        val history = ui.onNodeWithTag("timeline-entry-e13")
        history.assertIsDisplayed()
        ui.runOnIdle { height = 500.dp }
        history.assertIsDisplayed()

        ui.runOnIdle { screen = screen.copy(input = InputState(lastSend = "send-1")) }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()
        ui.runOnIdle { screen = screen.copy(entries = entries + TimelineItem("sent", "はる", "送信済み", "12:01")) }
        ui.onNodeWithTag("timeline-entry-sent").assertIsDisplayed()
    }

    @Test
    fun returningToAConversationRestoresTheSameMessageAfterEarlierRowsArrive() {
        var screen by mutableStateOf<AndroidScreen>(channel)
        ui.setContent {
            AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> }
        }

        val list = ui.onNodeWithTag("conversation-timeline")
        val history = ui.onNodeWithTag("timeline-entry-e13")
        list.performTouchInput { swipeDown() }
        list.performScrollToIndex(12)
        val originalTop = history.fetchSemanticsNode().boundsInRoot.top

        ui.runOnIdle { screen = AndroidScreen.Channels(emptyList()) }
        ui.onNodeWithText("チャンネル").assertIsDisplayed()
        val earlier = (1..5).map { TimelineItem("earlier-$it", "はる", "過去 $it", "11:00") }
        ui.runOnIdle { screen = channel.copy(entries = emptyList()) }
        ui.onNodeWithTag("conversation-timeline").assertDoesNotExist()
        ui.runOnIdle { screen = channel.copy(entries = earlier + entries) }

        assertEquals(originalTop, history.fetchSemanticsNode().boundsInRoot.top, 1f)
    }

    @Test
    fun returningToHomeAndChannelsRestoresTheirOwnScrollPositions() {
        val home = AndroidScreen.Home(entries)
        val channels = AndroidScreen.Channels(listOf(
            ChannelSection("参加中", (1..40).map { ChannelItem("c$it", "チャンネル $it") }),
        ))
        var screen by mutableStateOf<AndroidScreen>(home)
        ui.setContent {
            AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> }
        }

        val post = ui.onNodeWithTag("timeline-entry-e34")
        ui.onNode(hasScrollToNodeAction()).performScrollToNode(hasTestTag("timeline-entry-e34"))
        val postTop = post.fetchSemanticsNode().boundsInRoot.top

        ui.runOnIdle { screen = channels }
        ui.onNode(hasScrollToNodeAction()).performScrollToNode(hasText("チャンネル 34"))
        val channelRow = ui.onNodeWithText("チャンネル 34")
        val channelTop = channelRow.fetchSemanticsNode().boundsInRoot.top

        ui.runOnIdle { screen = channel }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()
        ui.runOnIdle { screen = channels }
        assertEquals(channelTop, channelRow.fetchSemanticsNode().boundsInRoot.top, 1f)

        ui.runOnIdle { screen = home }
        assertEquals(postTop, post.fetchSemanticsNode().boundsInRoot.top, 1f)
    }
}
