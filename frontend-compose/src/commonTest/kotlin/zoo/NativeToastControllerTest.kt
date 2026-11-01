@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package zoo

import androidx.compose.ui.graphics.vector.PathParser
import kotlin.test.*
import kotlinx.coroutines.test.*

class NativeToastControllerTest {
    @Test
    fun successAndErrorUseParsableSonnerAssetsWhileDefaultHasNoIcon() {
        assertNull(nativeToastIconPath(ToastKind.Default))
        for (kind in listOf(ToastKind.Success, ToastKind.Error)) {
            // Parsing SVG nodes is common code; creating an Android Path needs a device.
            assertTrue(
                PathParser().parsePathString(nativeToastIconPath(kind)!!).toNodes().isNotEmpty()
            )
        }
    }

    @Test
    fun normalLifetimeAndSameMessageReplacementUseIndependentSerials() = runTest {
        val controller = NativeToastController(backgroundScope) { testScheduler.currentTime }
        controller.show("saved", ToastKind.Success)
        runCurrent()
        val first = controller.current!!
        assertEquals(4_000L, first.durationMillis)
        advanceTimeBy(3_000)
        controller.show("saved", ToastKind.Success)
        runCurrent()
        val replacement = controller.current!!
        assertTrue(replacement.serial > first.serial)
        advanceTimeBy(1_000)
        runCurrent()
        assertEquals(
            replacement,
            controller.current,
            "The original deadline cannot clear its replacement",
        )
        controller.dismiss(first.serial)
        assertEquals(
            replacement,
            controller.current,
            "Stale dismissals cannot clear the current toast",
        )
        advanceTimeBy(3_000)
        runCurrent()
        assertNull(controller.current)
    }

    @Test
    fun hoverPreservesRemainingTimeInsteadOfRestartingTheLifetime() = runTest {
        val controller = NativeToastController(backgroundScope) { testScheduler.currentTime }
        controller.show("error", ToastKind.Error)
        runCurrent()
        val serial = controller.current!!.serial
        advanceTimeBy(1_000)
        controller.setHovered(serial, true)
        advanceTimeBy(20_000)
        runCurrent()
        assertNotNull(controller.current)
        controller.setHovered(serial, false)
        runCurrent()
        advanceTimeBy(2_999)
        runCurrent()
        assertNotNull(controller.current)
        advanceTimeBy(1)
        runCurrent()
        assertNull(controller.current)
    }

    @Test
    fun hiddenDocumentsPauseAndInfinityStoriesStayUntilDismissed() = runTest {
        val controller = NativeToastController(backgroundScope) { testScheduler.currentTime }
        controller.setVisible(false)
        controller.show("offline", ToastKind.Error)
        advanceTimeBy(10_000)
        runCurrent()
        assertNotNull(controller.current)
        controller.setVisible(true)
        runCurrent()
        advanceTimeBy(4_000)
        runCurrent()
        assertNull(controller.current)
        controller.show("storybook", ToastKind.Success, durationMillis = null)
        runCurrent()
        advanceTimeBy(100_000)
        runCurrent()
        assertEquals("storybook", controller.current?.message)
        assertNull(controller.current?.durationMillis)
        controller.dismiss()
        assertNull(controller.current)
    }

    @Test
    fun closingTheSessionOwnerCancelsItsTimerAndFutureNotifications() = runTest {
        val controller = NativeToastController(backgroundScope) { testScheduler.currentTime }
        controller.show("first")
        runCurrent()
        controller.close()
        controller.show("late async failure", ToastKind.Error)
        advanceTimeBy(10_000)
        runCurrent()
        assertNull(controller.current)
    }
}
