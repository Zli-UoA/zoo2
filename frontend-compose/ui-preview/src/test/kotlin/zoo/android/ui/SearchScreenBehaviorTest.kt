package zoo.android.ui

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import zoo.GlobalSearchType
import zoo.SearchQueryProblem

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "ja-rJP-w360dp-h760dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SearchScreenBehaviorTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    private val results = (1..12).map { SearchResultItem("message:m$it", "はる", "会議の結果 $it", "11/21 12:05", "開発") }

    private fun search(query: String, items: List<SearchResultItem> = results, canLoadOlder: Boolean = false) =
        AndroidScreen.Search(SearchScreenState(query, query, GlobalSearchType.All, items, loaded = true,
            canLoadOlder = canLoadOlder, searchedUntil = "2025-12"))

    private fun show(initial: AndroidScreen, actions: MutableList<AndroidUiAction> = mutableListOf()): (AndroidScreen) -> Unit {
        var screen by mutableStateOf(initial)
        ui.setContent { AndroidScreens(screen, dark = false, foreground = true, dispatch = { actions += it }, language = "ja") { _, _ -> } }
        return { next -> ui.runOnIdle { screen = next } }
    }

    @Test
    fun returningFromAResultKeepsTheListPositionAndOtherWordsStartAtTheTop() {
        val set = show(search("会議"))
        ui.onNodeWithTag("global-search-results").performScrollToNode(hasText("会議の結果 12"))
        set(AndroidScreen.Conversation("dev", "開発", "", emptyList(), InputState()))
        set(search("会議"))
        ui.onNodeWithText("会議の結果 12").assertIsDisplayed()
        ui.onNodeWithText("会議の結果 1").assertIsNotDisplayed()
        set(search("資料"))
        ui.onNodeWithText("会議の結果 1").assertIsDisplayed()
    }

    @Test
    fun resultsFoundInOlderMonthsStartAtTheNewest() {
        val set = show(search("会議", emptyList(), canLoadOlder = true))
        ui.onNodeWithText("さらに古い結果を検索").assertIsDisplayed()
        set(search("会議"))
        ui.onNodeWithText("会議の結果 1").assertIsDisplayed()
    }

    @Test
    fun theImeSearchKeySearchesAgain() {
        val actions = mutableListOf<AndroidUiAction>()
        show(search("会議"), actions)
        ui.onNodeWithTag("global-search-field").performImeAction()
        ui.runOnIdle { check(actions.last() == AndroidUiAction.SubmitSearch) { actions.toString() } }
    }

    @Test
    fun wordsTheServerRejectsExplainTheLimitWithoutRetry() {
        show(AndroidScreen.Search(SearchScreenState("a b c d e f", "a b c d e f", GlobalSearchType.All,
            problem = SearchQueryProblem.TooManyTerms)))
        ui.onNodeWithText("検索できる語は 5 個までです。語を減らしてください。").assertIsDisplayed()
        ui.onAllNodesWithText("再試行").assertCountEquals(0)
    }

    @Test
    fun openingTheSameMessageAgainScrollsToIt() {
        val entries = (1..30).map { TimelineItem("e$it", "はる", "メッセージ $it", "12:00") }
        fun conversation(request: String) =
            AndroidScreen.Conversation("dev", "開発", "", entries, InputState(), focusMessage = "e3", focusRequest = request)
        val set = show(conversation("message=e3&open=1"))
        ui.onNodeWithText("メッセージ 3").assertIsDisplayed()
        ui.onAllNodes(hasScrollToNodeAction()).onFirst().performScrollToNode(hasTestTag("timeline-entry-e30"))
        ui.onNodeWithText("メッセージ 3").assertIsNotDisplayed()
        set(conversation("message=e3&open=2"))
        ui.onNodeWithText("メッセージ 3").assertIsDisplayed()
    }
}
