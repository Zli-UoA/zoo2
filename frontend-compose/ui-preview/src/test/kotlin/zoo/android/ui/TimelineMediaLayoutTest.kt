package zoo.android.ui

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import zoo.GlobalSearchType

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class TimelineMediaLayoutTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private val images = List(6) { index ->
        TimelineMedia("key-$index", "photo-$index", TimelineMediaKind.Image,
            imageAspectRatio = if (index == 0) 0.5f else 2f)
    }

    @Test
    fun apubGridKeepsItsFootprintAndShowsTheExtraCount() {
        val count = mutableIntStateOf(1)
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        var opened: TimelineMedia? = null
        ui.setContent {
            CompositionLocalProvider(LocalCachedTimelineImage provides { bitmap }) {
                Box(Modifier.width(240.dp)) {
                    TimelineMediaGroup(images.take(count.intValue), AndroidColors(false), apub = true) { opened = it }
                }
            }
        }
        val grid = ui.onNodeWithTag("apub-image-grid")
        grid.assertHeightIsEqualTo(320.dp)
        ui.runOnIdle { count.intValue = 2 }
        grid.assertHeightIsEqualTo(135.dp)
        fun tile(index: Int) = ui.onNodeWithContentDescription("photo-$index")
        fun bounds(index: Int) = tile(index).getUnclippedBoundsInRoot()
        assertEquals(bounds(0).bottom - bounds(0).top, bounds(1).bottom - bounds(1).top)
        assertTrue(bounds(0).right < bounds(1).left)
        ui.runOnIdle { count.intValue = 3 }
        assertEquals(bounds(1).left, bounds(2).left)
        assertTrue(bounds(1).bottom < bounds(2).top)
        assertEquals(bounds(0).bottom - bounds(0).top, 135.dp)
        ui.runOnIdle { count.intValue = 4 }
        assertEquals(bounds(0).left, bounds(2).left)
        assertEquals(bounds(1).left, bounds(3).left)
        assertEquals(bounds(0).top, bounds(1).top)
        ui.runOnIdle { count.intValue = 6 }
        ui.onNodeWithText("+2").assertExists()
        tile(3).performClick()
        ui.runOnIdle { assertEquals(images[3], opened) }
    }

    @Test
    fun channelStripScrollsAndViewerStartsOnThePickedImage() {
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        var selection: TimelineMediaSelection? = null
        ui.setContent {
            CompositionLocalProvider(LocalCachedTimelineImage provides { bitmap }) {
                Box(Modifier.width(240.dp)) {
                    TimelineMediaGroup(images, AndroidColors(false), apub = false) {
                        selection = selectMediaPreview(images, it.key)
                    }
                }
            }
        }
        val strip = ui.onNodeWithTag("channel-image-strip")
        assertEquals(strip.getUnclippedBoundsInRoot().left,
            ui.onNodeWithContentDescription("photo-0").getUnclippedBoundsInRoot().left)
        strip.performScrollToIndex(5)
        ui.onNodeWithContentDescription("photo-5").assertIsDisplayed().performClick()
        ui.runOnIdle { assertEquals(5, selection?.initialPage) }
    }

    @Test
    fun summalyTruncatesSiteTitleAndDescription() {
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        val site = "Site whose name should not wrap on narrow screens"
        val title = "A title that is too long to fit on a single line"
        val description = "A long description of the linked article. ".repeat(12)
        ui.setContent {
            CompositionLocalProvider(LocalCachedTimelineImage provides { bitmap }) {
                Box(Modifier.width(180.dp)) {
                    EntryCards(listOf(EntryCard(title, description, "https://example.test", site,
                        image = images.first(), summaly = true)), AndroidColors(false))
                }
            }
        }
        fun textLayout(text: String): TextLayoutResult {
            val results = mutableListOf<TextLayoutResult>()
            ui.onNodeWithText(text).performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(results) }
            return results.single()
        }
        for ((text, lines) in listOf(site to 1, title to 1, description to 3)) {
            val layout = textLayout(text)
            assertEquals(lines, layout.layoutInput.maxLines)
            assertEquals(TextOverflow.Ellipsis, layout.layoutInput.overflow)
        }
    }

    @Test
    fun searchResultsReuseTheAvatarAndImageComponents() {
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        val actions = mutableListOf<AndroidUiAction>()
        val result = SearchResultItem("file-hit", "はる", "写真.png", "12:00", "写真部", file = true,
            avatar = "https://example.test/avatar.png", media = images.first())
        ui.setContent {
            CompositionLocalProvider(LocalCachedTimelineImage provides { bitmap }) {
                Column {
                    SearchPage(AndroidScreen.Search(SearchScreenState("写真", "写真", GlobalSearchType.Files,
                        results = listOf(result), loaded = true)), AndroidColors(false), actions::add)
                }
            }
        }
        ui.onNodeWithTag("timeline-avatar", useUnmergedTree = true).assertIsDisplayed()
        ui.onNodeWithContentDescription("photo-0").assertIsDisplayed().performClick()
        ui.runOnIdle { assertEquals(AndroidUiAction.OpenSearchMedia("file-hit"), actions.last()) }
    }

    @Test
    fun viewerRetriesAnUnavailableImageAndOpensTheOriginal() {
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        val image = images.first().copy(attachmentId = "original")
        var loads = 0
        var opened: TimelineMedia? = null
        ui.setContent {
            CompositionLocalProvider(LocalTimelineImage provides { if (++loads == 1) null else bitmap }) {
                TimelineImagePages(TimelineMediaSelection(listOf(image), 0), openOriginal = { opened = it }) {}
            }
        }
        ui.onNodeWithText("画像を読み込めませんでした").assertIsDisplayed()
        ui.onNodeWithContentDescription("再試行").performClick()
        ui.onNodeWithContentDescription(image.name).assertIsDisplayed()
        ui.onNodeWithContentDescription("元のファイル").performClick()
        ui.runOnIdle {
            assertEquals(2, loads)
            assertEquals(image, opened)
        }
    }

    @Test
    fun mixedMediaKeepsAttachmentOrderAndUsesDecodedRatioWhenDimensionsAreMissing() {
        val image = images.first().copy(imageAspectRatio = null)
        val bitmap = Bitmap.createBitmap(24, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        val video = TimelineMedia("video", "video", TimelineMediaKind.Video)
        ui.setContent {
            CompositionLocalProvider(LocalCachedTimelineImage provides { bitmap }) {
                Column(Modifier.width(240.dp)) {
                    TimelineMediaGroup(listOf(video, image, video.copy(key = "second")), AndroidColors(false), apub = true) {}
                }
            }
        }
        val videos = ui.onAllNodesWithText("動画を再生")
        val preview = ui.onNodeWithTag("apub-image-grid").assertHeightIsEqualTo(135.dp).getUnclippedBoundsInRoot()
        val first = videos[0].getUnclippedBoundsInRoot()
        val last = videos[1].getUnclippedBoundsInRoot()
        assertTrue(first.bottom <= preview.top && preview.bottom <= last.top)
    }

    @Test
    fun revealingSensitiveImagesKeepsTheirFrameAndDoesNotAddAnEyeToTheViewer() {
        val bitmap = Bitmap.createBitmap(8, 24, Bitmap.Config.ARGB_8888).asImageBitmap()
        val sensitive = images[1].copy(sensitive = true, imageAspectRatio = null)
        var loads = 0
        val viewing = mutableStateOf(false)
        ui.setContent {
            CompositionLocalProvider(LocalTimelineImage provides { loads++; bitmap }) {
                if (viewing.value) {
                    TimelineImagePages(TimelineMediaSelection(listOf(images[0], sensitive), 0)) {}
                } else {
                    Column(Modifier.width(240.dp)) {
                        TimelineMediaGroup(listOf(sensitive), AndroidColors(false), apub = true) {}
                    }
                }
            }
        }
        val grid = ui.onNodeWithTag("apub-image-grid")
        grid.assertHeightIsEqualTo(180.dp)
        ui.onNodeWithContentDescription("画像を表示").performClick()
        grid.assertHeightIsEqualTo(180.dp)
        ui.onNodeWithContentDescription("画像を隠す").assertExists()
        ui.runOnIdle { viewing.value = true }
        val viewer = ui.onNodeWithTag("timeline-image-viewer")
        viewer.performTouchInput { swipeLeft() }
        ui.runOnIdle { assertEquals(2, loads) }
        ui.onNodeWithContentDescription("画像を表示").performClick()
        ui.onNodeWithTag("viewer-image-1").assertIsDisplayed()
        ui.onNodeWithContentDescription("画像を隠す").assertDoesNotExist()
        ui.runOnIdle { assertEquals(3, loads) }
    }

    @Test
    fun viewerSwipesBetweenImagesAndDismissesVertically() {
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        var dismissed = false
        ui.setContent {
            CompositionLocalProvider(LocalCachedTimelineImage provides { bitmap }) {
                TimelineImagePages(TimelineMediaSelection(images.take(3), 1)) { dismissed = true }
            }
        }
        val viewer = ui.onNodeWithTag("timeline-image-viewer")
        ui.onNodeWithTag("viewer-image-1").assertIsDisplayed()
        viewer.performTouchInput { swipeLeft() }
        ui.onNodeWithTag("viewer-image-2").assertIsDisplayed()
        ui.runOnIdle { assertTrue(!dismissed) }
        viewer.performTouchInput { swipeDown() }
        ui.runOnIdle { assertTrue(dismissed) }
    }
}
