package zoo.android.ui

import androidx.activity.ComponentActivity
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.StateRestorationTester
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
import zoo.CompletionOption

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "ja-rJP-w360dp-h760dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ConversationScrollTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private val entries = (1..40).map { TimelineItem("e$it", "はる", "メッセージ $it", "12:00") }
    private val channel = AndroidScreen.Conversation("dev", "開発", "", entries, InputState())

    @Test
    fun mentionSuggestionShowsTheDisplayNameAndSelectsTheUsername() {
        var chosen: String? = null
        ui.setContent {
            ConversationInput(
                InputState("@ali", mentions = listOf(CompletionOption("alice", "アリス"))),
                AndroidColors(false), textChanged = {}, onTool = {}, onMention = { chosen = it },
            )
        }

        ui.onNode(SemanticsMatcher.keyIsDefined(SemanticsActions.SetText)).performClick().assertIsFocused()
        ui.onNodeWithText("アリス").assertIsDisplayed()
        ui.onNodeWithText("@alice").assertIsSelected().performClick()
        ui.runOnIdle { assertEquals("alice", chosen) }
    }

    @Test
    fun hiddenMentionSuggestionsKeepTheSendActionVisibleAndDoNotConsumeEnter() {
        var chosen: String? = null
        ui.setContent {
            ConversationInput(
                InputState("first line\nsecond line\n@", notices = listOf("配布先の確認", "確認が必要な端末"),
                    mentions = (1..8).map { CompletionOption("user$it", "User $it") }),
                AndroidColors(false), textChanged = {}, onTool = {}, onMention = { chosen = it },
                modifier = Modifier.height(160.dp),
            )
        }
        val editor = ui.onNode(SemanticsMatcher.keyIsDefined(SemanticsActions.SetText))
        editor.performClick().assertIsDisplayed()
        ui.onNodeWithContentDescription("送信").assertIsDisplayed().assertIsEnabled()
        ui.onNodeWithText("@user1").assertIsNotDisplayed()
        editor.performKeyInput { pressKey(Key.Enter) }
        ui.runOnIdle { assertEquals(null, chosen) }
    }

    @Test
    fun reopeningAChannelShowsTheUnreadDividerAndReportsReadOnlyInTheForeground() {
        val unreadEntry = entries[10].copy(dateDivider = "昨日", unreadDivider = "新着メッセージ")
        var foreground by mutableStateOf(false)
        var screen by mutableStateOf<AndroidScreen>(channel)
        val written = mutableListOf<String>()
        ui.setContent { AndroidScreens(screen, false, foreground, dispatch = {}) { _, _ -> } }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()
        ui.runOnIdle { screen = AndroidScreen.Channels(emptyList()) }
        ui.waitForIdle()

        ui.runOnIdle {
            screen = channel.copy(
                entries = entries.toMutableList().apply { set(10, unreadEntry) },
                unreadPositionPending = true,
                onUnreadPositioned = {
                    screen = (screen as AndroidScreen.Conversation).copy(onUnreadPositioned = null)
                },
                onReadThrough = { id -> if (id != null) written += id },
            )
        }
        ui.runOnIdle {
            screen = (screen as AndroidScreen.Conversation).copy(unreadPositionPending = false)
        }
        ui.onNodeWithText("昨日").assertIsDisplayed()
        ui.onNodeWithText("新着メッセージ").assertIsDisplayed()
        ui.runOnIdle { assertTrue(written.isEmpty()) }
        ui.onNodeWithTag("conversation-timeline").performScrollToIndex(entries.lastIndex)
        ui.runOnIdle {
            assertTrue(written.isEmpty())
            foreground = true
        }
        ui.runOnIdle { assertEquals(listOf(entries.last().id), written) }
        ui.onNodeWithTag("conversation-timeline").performScrollToIndex(10)
        ui.onNodeWithText("新着メッセージ").assertIsDisplayed()
    }

    @Test
    fun aRestoredConversationReportsNewMessagesAtTheEndAfterSync() {
        val reported = mutableListOf<String>()
        var screen by mutableStateOf<AndroidScreen>(channel)
        ui.setContent { AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> } }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()
        ui.runOnIdle { screen = AndroidScreen.Channels(emptyList()) }
        ui.waitForIdle()

        ui.runOnIdle {
            screen = channel.copy(
                historyReady = false,
                unreadPositionPending = true,
                onUnreadPositioned = {
                    screen = (screen as AndroidScreen.Conversation).copy(onUnreadPositioned = null)
                },
                onReadThrough = { id -> if (id != null) reported.add(id) },
            )
        }
        ui.onNodeWithTag("timeline-entry-e40").assertExists()
        ui.runOnIdle {
            assertTrue(reported.isEmpty())
            screen = (screen as AndroidScreen.Conversation).copy(
                entries = entries + TimelineItem("e41", "はる", "同期した投稿", "12:01"),
                historyReady = true,
                unreadPositionPending = false,
            )
        }
        ui.onNodeWithTag("timeline-entry-e41").assertIsDisplayed()
        ui.runOnIdle { assertEquals("e41", reported.last()) }
    }

    @Test
    fun focusingAnOlderMessageDoesNotReportThePreviousEndLayout() {
        val reported = mutableListOf<String>()
        var foreground by mutableStateOf(false)
        var screen by mutableStateOf(channel.copy(onReadThrough = { id -> if (id != null) reported.add(id) }))
        ui.setContent { AndroidScreens(screen, false, foreground, dispatch = {}) { _, _ -> } }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()

        ui.runOnIdle {
            screen = screen.copy(focusMessage = "e3", focusRequest = "open-e3")
            foreground = true
        }
        ui.onNodeWithTag("timeline-entry-e4").assertIsDisplayed()
        ui.runOnIdle { assertTrue(reported.isEmpty()) }
        ui.onNodeWithTag("conversation-timeline").performScrollToIndex(entries.lastIndex)
        ui.runOnIdle { assertEquals("e40", reported.last()) }
    }

    @Test
    fun incompleteUnreadHistoryIsShownBelowTheConversationHeader() {
        ui.setContent {
            AndroidScreens(channel.copy(unreadHistoryIncomplete = true), false, true, dispatch = {}) { _, _ -> }
        }
        val title = ui.onNodeWithText("開発")
        val notice = ui.onNodeWithText("これより古い未読は読み込まれていません")
        notice.assertIsDisplayed()
        assertTrue(notice.fetchSemanticsNode().boundsInRoot.top > title.fetchSemanticsNode().boundsInRoot.bottom)
    }

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
        compact.performTouchInput { click() }
        compact.assertIsDisplayed()
        val list = ui.onNodeWithTag("conversation-timeline")
        val viewport = list.fetchSemanticsNode().boundsInRoot
        val last = ui.onNodeWithTag("timeline-entry-e40")
        val lastTop = last.fetchSemanticsNode().positionInRoot.y
        val compactTop = compact.fetchSemanticsNode().boundsInRoot.top
        val delta = with(ui.density) { 120.dp.toPx() }
        compact.performTouchInput { down(center); moveBy(Offset(0f, -delta)) }
        assertTrue(compact.fetchSemanticsNode().boundsInRoot.top < compactTop - delta / 2)
        assertEquals(viewport, list.fetchSemanticsNode().boundsInRoot)
        assertEquals(lastTop, last.fetchSemanticsNode().positionInRoot.y, 1f)
        compact.performTouchInput { up() }

        val expanded = ui.onNodeWithContentDescription("入力欄を縮める")
        val expandedTop = expanded.fetchSemanticsNode().boundsInRoot.top
        expanded.performTouchInput { down(center); moveBy(Offset(0f, delta)) }
        assertTrue(expanded.fetchSemanticsNode().boundsInRoot.top > expandedTop + delta / 2)
        expanded.performTouchInput { cancel() }
        assertEquals(expandedTop, expanded.fetchSemanticsNode().boundsInRoot.top, 1f)

        expanded.performTouchInput { down(center); moveBy(Offset(0f, delta)); up() }
        compact.assertIsDisplayed()
        assertEquals(viewport, list.fetchSemanticsNode().boundsInRoot)
        assertEquals(lastTop, last.fetchSemanticsNode().positionInRoot.y, 1f)
    }

    @Test
    fun theHandleSupportsPhysicalKeyboardExpansionAndCollapse() {
        var expanded by mutableStateOf(false)
        ui.setContent {
            ConversationInput(InputState("下書き", expanded = expanded), AndroidColors(false), textChanged = {},
                onTool = { if (it == InputTool.Expand) expanded = !expanded })
        }

        val compact = ui.onNodeWithContentDescription("入力欄を広げる")
        compact.performSemanticsAction(SemanticsActions.RequestFocus)
        compact.performKeyInput { pressKey(Key.Enter) }

        val expandedHandle = ui.onNodeWithContentDescription("入力欄を縮める")
        expandedHandle.performSemanticsAction(SemanticsActions.RequestFocus)
        expandedHandle.performKeyInput { pressKey(Key.Spacebar) }
        compact.assertIsDisplayed()
        compact.performSemanticsAction(SemanticsActions.RequestFocus)
        compact.performKeyInput { pressKey(Key.DirectionCenter) }
        expandedHandle.performSemanticsAction(SemanticsActions.RequestFocus)
        expandedHandle.performKeyInput { pressKey(Key.NumPadEnter) }
        compact.assertIsDisplayed()
    }

    @Test
    fun expandingRemovesTheCoveredAttachmentFromKeyboardFocus() {
        val item = entries.first().copy(attachments = listOf("file" to "添付ファイル"))
        var screen by mutableStateOf(channel.copy(entries = listOf(item), input = InputState("下書き")))
        ui.setContent { AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> } }

        val attachment = ui.onNodeWithText("添付ファイル", useUnmergedTree = true)
        attachment.assert(SemanticsMatcher.keyIsDefined(SemanticsActions.RequestFocus))
        ui.runOnIdle { screen = screen.copy(input = screen.input.copy(expanded = true)) }
        attachment.assertDoesNotExist()
    }

    @Test
    fun expandedInputCollapsesEvenWhenTheHandleHasNoRoomToMove() {
        var expanded by mutableStateOf(true)
        ui.setContent {
            Box(Modifier.height(112.dp)) {
                ConversationInput(InputState("下書き", expanded = expanded), AndroidColors(false),
                    textChanged = {}, onTool = { if (it == InputTool.Expand) expanded = false })
            }
        }

        val handle = ui.onNodeWithContentDescription("入力欄を縮める")
        val top = handle.fetchSemanticsNode().boundsInRoot.top
        val distance = with(ui.density) { 90.dp.toPx() }
        handle.performTouchInput { down(center); moveBy(Offset(0f, distance)) }
        assertEquals(top, handle.fetchSemanticsNode().boundsInRoot.top, 1f)
        handle.performTouchInput { cancel() }
        handle.performTouchInput { down(center); moveBy(Offset(0f, distance / 3)); up() }
        handle.assertIsDisplayed()

        handle.performTouchInput { down(center); moveBy(Offset(0f, distance)); up() }
        ui.onNodeWithContentDescription("入力欄を広げる").assertIsDisplayed()
    }

    @Test
    fun hidingAndRevealingTheHeaderDoesNotResizeTheList() {
        ui.setContent { AndroidScreens(channel, false, true, dispatch = {}) { _, _ -> } }

        val list = ui.onNodeWithTag("conversation-timeline")
        val header = ui.onNodeWithText("開発")
        header.assertIsDisplayed()
        val viewport = list.fetchSemanticsNode().boundsInRoot
        list.performTouchInput { swipeDown(startY = height * .25f) }
        list.performTouchInput { swipeDown(startY = height * .25f) }
        list.performTouchInput { swipeUp() }
        header.assertDoesNotExist()
        assertEquals(viewport, list.fetchSemanticsNode().boundsInRoot)
        list.performTouchInput { swipeDown(startY = height * .25f) }
        header.assertIsDisplayed()
        assertEquals(viewport, list.fetchSemanticsNode().boundsInRoot)
    }

    @Test
    fun theOverlayHeaderBlocksTapsOnCoveredContentAndKeepsItsButtonWorking() {
        var contentTaps = 0
        var headerButtonTaps = 0
        ui.setContent {
            CollapsingHeader(header = {
                Box(Modifier.fillMaxWidth().height(56.dp).testTag("overlay-header")) {
                    Box(Modifier.size(48.dp).clickable { headerButtonTaps++ }.testTag("header-button"))
                }
            }) { _, _ ->
                Box(Modifier.fillMaxSize().clickable { contentTaps++ }.testTag("behind-header"))
            }
        }

        ui.onNodeWithTag("overlay-header", useUnmergedTree = true).performTouchInput { click(center) }
        ui.onNodeWithTag("header-button").performTouchInput { click() }
        ui.runOnIdle {
            assertEquals(0, contentTaps)
            assertEquals(1, headerButtonTaps)
        }
        ui.onNodeWithTag("behind-header").performTouchInput { click(center) }
        ui.runOnIdle { assertEquals(1, contentTaps) }
    }

    @OptIn(ExperimentalFoundationApi::class)
    @Test
    fun bringingATallRowIntoViewAlignsItsTopBelowTheHeader() {
        val overlay = 56.dp
        var reveal by mutableStateOf(false)
        ui.setContent {
            val initialScroll = with(LocalDensity.current) { 80.dp.roundToPx() }
            val scroll = rememberLazyListState(initialFirstVisibleItemScrollOffset = initialScroll)
            val requester = remember { BringIntoViewRequester() }
            LaunchedEffect(reveal) { if (reveal) requester.bringIntoView() }
            WithHeaderBringIntoView(overlay) {
                LazyColumn(Modifier.height(500.dp), state = scroll) {
                    item { Spacer(Modifier.height(110.dp)) }
                    item { Box(Modifier.size(48.dp, 480.dp).bringIntoViewRequester(requester).testTag("covered-control")) }
                    item { Spacer(Modifier.height(700.dp)) }
                }
            }
        }

        val control = ui.onNodeWithTag("covered-control")
        val headerBottom = with(ui.density) { overlay.toPx() }
        assertTrue(control.fetchSemanticsNode().positionInRoot.y < headerBottom)
        ui.runOnIdle { reveal = true }
        assertEquals(headerBottom, control.fetchSemanticsNode().positionInRoot.y, 1f)
    }

    @Test
    fun aSearchResultOpenedInTheFullComposerTakesPriorityOverImeTracking() {
        var imeHeight by mutableStateOf(0.dp)
        var expanded by mutableStateOf(false)
        var target by mutableStateOf<String?>(null)
        ui.setContent {
            val imePixels = with(LocalDensity.current) { imeHeight.roundToPx() }
            Column(Modifier.height(700.dp - imeHeight)) {
                TimelineWithInput(entries, InputState(expanded = expanded), AndroidColors(false), dispatch = {},
                    allowThreads = true, focusMessage = target, imeInsets = WindowInsets(bottom = imePixels))
            }
        }

        val list = ui.onNodeWithTag("conversation-timeline")
        list.performTouchInput { swipeDown() }
        list.performScrollToIndex(12)
        ui.runOnIdle { expanded = true }
        list.assertDoesNotExist()
        ui.runOnIdle { target = "e3" }
        ui.waitForIdle()
        ui.runOnIdle { imeHeight = 120.dp }
        ui.waitForIdle()
        ui.runOnIdle { expanded = false }
        val result = ui.onNodeWithTag("timeline-entry-e3")
        result.assertIsDisplayed()
        ui.runOnIdle { imeHeight = 0.dp }
        result.assertIsDisplayed()
    }

    @Test
    fun openingTheImeMovesVisibleHistoryWithoutJumpingToTheEnd() {
        var imeHeight by mutableStateOf(0.dp)
        ui.setContent {
            val imePixels = with(LocalDensity.current) { imeHeight.roundToPx() }
            Column(Modifier.height(700.dp - imeHeight)) {
                TimelineWithInput(entries, InputState(), AndroidColors(false), dispatch = {},
                    allowThreads = true, imeInsets = WindowInsets(bottom = imePixels))
            }
        }

        val list = ui.onNodeWithTag("conversation-timeline")
        list.performTouchInput { swipeDown() }
        list.performScrollToIndex(12)
        val history = ui.onNodeWithTag("timeline-entry-e17")
        val originalTop = history.fetchSemanticsNode().positionInRoot.y
        val opening = 120.dp
        val openingPixels = with(ui.density) { opening.toPx() }
        ui.runOnIdle { imeHeight = opening }
        assertEquals(originalTop - openingPixels, history.fetchSemanticsNode().positionInRoot.y, 1f)
        ui.onNodeWithTag("timeline-entry-e40").assertIsNotDisplayed()
        ui.runOnIdle { imeHeight = 0.dp }
        assertEquals(originalTop, history.fetchSemanticsNode().positionInRoot.y, 1f)

        ui.runOnIdle { imeHeight = opening }
        list.performTouchInput { swipeDown(startY = height * .25f) }
        list.performScrollToIndex(8)
        val older = ui.onNodeWithTag("timeline-entry-e9")
        val afterScroll = older.fetchSemanticsNode().positionInRoot.y
        ui.runOnIdle { imeHeight = 0.dp }
        assertEquals(afterScroll, older.fetchSemanticsNode().positionInRoot.y, 1f)
    }

    @Test
    fun closingTheImeWhileExpandedRestoresHistoryWhenTheComposerCollapses() {
        var imeHeight by mutableStateOf(0.dp)
        var expanded by mutableStateOf(false)
        ui.setContent {
            val imePixels = with(LocalDensity.current) { imeHeight.roundToPx() }
            Column(Modifier.height(700.dp - imeHeight)) {
                TimelineWithInput(entries, InputState(expanded = expanded), AndroidColors(false), dispatch = {},
                    allowThreads = true, imeInsets = WindowInsets(bottom = imePixels))
            }
        }

        val list = ui.onNodeWithTag("conversation-timeline")
        list.performTouchInput { swipeDown() }
        list.performScrollToIndex(12)
        val history = ui.onNodeWithTag("timeline-entry-e17")
        val originalTop = history.fetchSemanticsNode().positionInRoot.y
        ui.runOnIdle { imeHeight = 120.dp }
        assertTrue(history.fetchSemanticsNode().positionInRoot.y < originalTop)

        ui.runOnIdle { expanded = true }
        list.assertDoesNotExist()
        ui.runOnIdle { imeHeight = 0.dp }
        ui.runOnIdle { expanded = false }
        assertEquals(originalTop, history.fetchSemanticsNode().positionInRoot.y, 1f)

        ui.runOnIdle { imeHeight = 120.dp }
        val keyboardTop = history.fetchSemanticsNode().positionInRoot.y
        ui.runOnIdle { expanded = true }
        list.assertDoesNotExist()
        ui.runOnIdle { imeHeight = 0.dp }
        ui.waitForIdle()
        ui.runOnIdle { imeHeight = 120.dp }
        ui.waitForIdle()
        ui.runOnIdle { expanded = false }
        assertEquals(keyboardTop, history.fetchSemanticsNode().positionInRoot.y, 1f)
    }

    @Test
    fun openingTheImeInTheFullComposerAdjustsHistoryAfterCollapse() {
        var imeHeight by mutableStateOf(0.dp)
        var expanded by mutableStateOf(false)
        var rows by mutableStateOf(entries)
        ui.setContent {
            val imePixels = with(LocalDensity.current) { imeHeight.roundToPx() }
            Column(Modifier.height(700.dp - imeHeight)) {
                TimelineWithInput(rows, InputState(expanded = expanded), AndroidColors(false), dispatch = {},
                    allowThreads = true, imeInsets = WindowInsets(bottom = imePixels))
            }
        }

        val list = ui.onNodeWithTag("conversation-timeline")
        list.performTouchInput { swipeDown() }
        list.performScrollToIndex(12)
        val history = ui.onNodeWithTag("timeline-entry-e17")
        val originalTop = history.fetchSemanticsNode().positionInRoot.y
        ui.runOnIdle { expanded = true }
        list.assertDoesNotExist()
        ui.runOnIdle {
            rows = (1..5).map { TimelineItem("earlier-$it", "はる", "過去 $it", "11:00") } + entries
        }
        list.assertDoesNotExist()
        val opening = 120.dp
        val openingPixels = with(ui.density) { opening.toPx() }
        ui.runOnIdle { imeHeight = opening }
        ui.waitForIdle()
        ui.runOnIdle { expanded = false }
        assertEquals(originalTop - openingPixels, history.fetchSemanticsNode().positionInRoot.y, 1f)
    }

    @Test
    fun restoringTheScreenWhileTheImeIsOpenKeepsTheOriginalHistoryAnchor() {
        var imeHeight by mutableStateOf(0.dp)
        val restoration = StateRestorationTester(ui)
        restoration.setContent {
            val imePixels = with(LocalDensity.current) { imeHeight.roundToPx() }
            Column(Modifier.height(700.dp - imeHeight)) {
                TimelineWithInput(entries, InputState(), AndroidColors(false), dispatch = {},
                    allowThreads = true, imeInsets = WindowInsets(bottom = imePixels))
            }
        }

        val list = ui.onNodeWithTag("conversation-timeline")
        list.performTouchInput { swipeDown() }
        list.performScrollToIndex(12)
        val history = ui.onNodeWithTag("timeline-entry-e17")
        val originalTop = history.fetchSemanticsNode().positionInRoot.y
        ui.runOnIdle { imeHeight = 120.dp }
        val withKeyboard = history.fetchSemanticsNode().positionInRoot.y
        restoration.emulateSavedInstanceStateRestore()
        assertEquals(withKeyboard, history.fetchSemanticsNode().positionInRoot.y, 1f)
        ui.runOnIdle { imeHeight = 0.dp }
        assertEquals(originalTop, history.fetchSemanticsNode().positionInRoot.y, 1f)
    }

    @Test
    fun openingTheImeNearTheEndWaitsForTheListToHaveRoom() {
        var imeHeight by mutableStateOf(0.dp)
        var viewportHeight by mutableStateOf(700.dp)
        ui.setContent {
            val imePixels = with(LocalDensity.current) { imeHeight.roundToPx() }
            Column(Modifier.height(viewportHeight)) {
                TimelineWithInput(entries, InputState(), AndroidColors(false), dispatch = {},
                    allowThreads = true, imeInsets = WindowInsets(bottom = imePixels))
            }
        }

        val list = ui.onNodeWithTag("conversation-timeline")
        val opening = 120.dp
        val openingPixels = with(ui.density) { opening.toPx() }
        list.performSemanticsAction(SemanticsActions.ScrollBy) { it(0f, -openingPixels / 4f) }
        val message = ui.onNodeWithTag("timeline-entry-e38")
        val originalTop = message.fetchSemanticsNode().positionInRoot.y
        ui.runOnIdle { imeHeight = opening }
        list.assertIsDisplayed()
        ui.runOnIdle { viewportHeight -= opening }
        assertEquals(originalTop - openingPixels, message.fetchSemanticsNode().positionInRoot.y, 1f)

        ui.runOnIdle { viewportHeight += opening }
        list.assertIsDisplayed()
        ui.runOnIdle { imeHeight = 0.dp }
        assertEquals(originalTop, message.fetchSemanticsNode().positionInRoot.y, 1f)
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
        fun assertAtEnd(id: String) {
            val last = ui.onNodeWithTag("timeline-entry-$id").assertIsDisplayed().fetchSemanticsNode()
            assertEquals(list.fetchSemanticsNode().boundsInRoot.bottom, last.positionInRoot.y + last.size.height, 1f)
        }
        assertAtEnd("e40")
        ui.onNode(hasSetTextAction()).performClick()
        assertAtEnd("e40")
        ui.runOnIdle { height = 430.dp }
        assertAtEnd("e40")

        list.performTouchInput { swipeDown() }
        list.performScrollToIndex(12)
        val history = ui.onNodeWithTag("timeline-entry-e13")
        history.assertIsDisplayed()
        ui.runOnIdle { height = 500.dp }
        history.assertIsDisplayed()

        ui.runOnIdle { screen = screen.copy(input = InputState(lastSend = "send-1")) }
        assertAtEnd("e40")
        ui.runOnIdle { screen = screen.copy(entries = entries + TimelineItem("sent", "はる", "送信済み", "12:01")) }
        assertAtEnd("sent")
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
        list.assertDoesNotExist()
        val earlier = (1..5).map { TimelineItem("earlier-$it", "はる", "過去 $it", "11:00") }
        ui.runOnIdle { screen = channel.copy(entries = emptyList()) }
        ui.onNodeWithTag("conversation-timeline").assertDoesNotExist()
        ui.runOnIdle { screen = channel.copy(entries = earlier + entries) }

        assertEquals(originalTop, history.fetchSemanticsNode().boundsInRoot.top, 1f)
    }

    @Test
    fun openingAThreadKeepsItsRootVisibleWhenRepliesArrive() {
        val root = TimelineItem("root", "はる", "親メッセージ", "12:00")
        var screen by mutableStateOf(AndroidScreen.Thread("開発", root, emptyList(), InputState(), historyReady = false))
        ui.setContent { AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> } }

        val parent = ui.onNodeWithTag("timeline-entry-root")
        parent.assertIsDisplayed()
        ui.runOnIdle { screen = screen.copy(replies = entries, historyReady = true) }
        parent.assertIsDisplayed()
    }

    @Test
    fun aRequestedMessageTakesPriorityOverAnUnseenSend() {
        var screen by mutableStateOf<AndroidScreen>(channel)
        ui.setContent { AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> } }

        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()
        ui.runOnIdle {
            screen = channel.copy(entries = entries.drop(10), input = InputState(lastSend = "send-1"),
                focusMessage = "e3", focusRequest = "open-e3")
        }
        ui.onNodeWithTag("timeline-entry-e3").assertDoesNotExist()
        ui.runOnIdle {
            screen = channel.copy(input = InputState(lastSend = "send-1"), focusMessage = "e3", focusRequest = "open-e3")
        }
        ui.onNodeWithTag("timeline-entry-e3").assertIsDisplayed()
    }

    @Test
    fun aLaterSendFollowsTheEndWhenTheRequestedMessageIsUnavailable() {
        val initial = channel.copy(focusMessage = "missing", focusRequest = "open-missing")
        var screen by mutableStateOf(initial)
        ui.setContent { AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> } }

        ui.onNodeWithTag("timeline-entry-e1").assertIsDisplayed()
        ui.runOnIdle { screen = initial.copy(input = InputState(lastSend = "new-send")) }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()

        ui.runOnIdle {
            screen = initial.copy(entries = listOf(TimelineItem("missing", "はる", "遅れて届いた行", "11:59")) + entries,
                input = InputState(lastSend = "new-send"))
        }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()
    }

    @Test
    fun sendingFromTheChannelWorksAfterReturningFromAFocusedReply() {
        val request = "open-reply"
        val focusedChannel = channel.copy(focusMessage = "reply", focusRequest = request)
        val reply = TimelineItem("reply", "はる", "指定された返信", "12:01")
        var screen by mutableStateOf<AndroidScreen>(focusedChannel)
        ui.setContent { AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> } }

        ui.onNodeWithTag("timeline-entry-e1").assertIsDisplayed()
        ui.runOnIdle {
            screen = AndroidScreen.Thread("開発", entries.first(), listOf(reply), InputState(),
                focusMessage = reply.id, focusRequest = request)
        }
        ui.onNodeWithTag("timeline-entry-reply").assertIsDisplayed()
        ui.runOnIdle { screen = focusedChannel }
        ui.onNodeWithTag("timeline-entry-e1").assertIsDisplayed()
        ui.runOnIdle { screen = focusedChannel.copy(input = InputState(lastSend = "channel-send")) }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()
    }

    @Test
    fun missingSavedMessageReturnsToTheEndAfterHistoryLoads() {
        var screen by mutableStateOf<AndroidScreen>(channel)
        ui.setContent { AndroidScreens(screen, false, true, dispatch = {}) { _, _ -> } }
        val list = ui.onNodeWithTag("conversation-timeline")
        list.performTouchInput { swipeDown() }
        list.performScrollToIndex(12)
        ui.onNodeWithTag("timeline-entry-e13").assertIsDisplayed()

        ui.runOnIdle { screen = AndroidScreen.Channels(emptyList()) }
        list.assertDoesNotExist()
        ui.runOnIdle { screen = channel.copy(entries = entries.drop(18), historyReady = false) }
        list.assertIsDisplayed()
        ui.runOnIdle { screen = channel.copy(entries = entries.drop(18), historyReady = true) }
        ui.onNodeWithTag("timeline-entry-e40").assertIsDisplayed()
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
