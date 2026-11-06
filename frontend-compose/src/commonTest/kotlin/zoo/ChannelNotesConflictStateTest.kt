package zoo

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class ChannelNotesConflictStateTest {
    @Test
    fun messageReadStateIsDisabledWhileANoteIsSelected() {
        assertTrue(shouldTrackChannelReadState(null))
        assertFalse(shouldTrackChannelReadState("note-1"))
    }

    @Test
    fun summaryPollingRunsOnMessageTabButSkipsAnInFlightLoadOrMutation() {
        assertTrue(shouldPollChannelNoteSummary(loadInFlight = false, busy = false))
        assertFalse(shouldPollChannelNoteSummary(loadInFlight = false, busy = true))
        assertFalse(shouldPollChannelNoteSummary(loadInFlight = true, busy = false))
    }

    @Test
    fun latestFetchRetryRemainsAvailableWhenTheGeneralErrorIsCleared() {
        val latestFetch = ChannelNoteLatestFetchState.Empty.markFailed(
            noteId = "note-1",
            error = "latest note could not be loaded",
            forDelete = true,
        )
        val generalErrorAfterSuccessfulPoll = ""

        assertTrue(generalErrorAfterSuccessfulPoll.isEmpty())
        assertTrue(latestFetch.canRetry(routeNoteId = "note-1", selectedNoteId = "note-1"))
    }

    @Test
    fun latestFetchRetryIsScopedToTheCurrentRouteAndSelection() {
        val latestFetch = ChannelNoteLatestFetchState.Empty.markFailed("note-1", "load failed", forDelete = false)

        assertFalse(latestFetch.canRetry(routeNoteId = "note-2", selectedNoteId = "note-1"))
        assertFalse(latestFetch.canRetry(routeNoteId = "note-1", selectedNoteId = "note-2"))
        assertFalse(latestFetch.canRetry(routeNoteId = null, selectedNoteId = "note-1"))
    }

    @Test
    fun switchingNotesClearsThePreviousNoteFetchFailure() {
        val latestFetch = ChannelNoteLatestFetchState.Empty.markFailed("note-1", "load failed", forDelete = true)

        val switched = latestFetch.forCurrentNote("note-2")

        assertFalse(switched.canRetry(routeNoteId = "note-2", selectedNoteId = "note-2"))
        assertFalse(switched.forDelete)
    }

    @Test
    fun cancellingDeleteConflictClearsTheRetryContext() {
        val latestFetch = ChannelNoteLatestFetchState.Empty.markFailed("note-1", "load failed", forDelete = true)

        val cancelled = latestFetch.clear()

        assertFalse(cancelled.canRetry(routeNoteId = "note-1", selectedNoteId = "note-1"))
        assertFalse(cancelled.forDelete)
        assertTrue(cancelled.error.isEmpty())
    }

    @Test
    fun deleteConfirmationCannotDeleteAfterRouteOrSelectionChanges() {
        val confirmation = ChannelNoteDeleteConfirmation.forNote("note-1")

        assertEquals("note-1", confirmation.confirmableNoteId("note-1", "note-1"))
        assertNull(confirmation.confirmableNoteId("note-2", "note-2"))
        assertNull(confirmation.confirmableNoteId("note-1", "note-2"))
        assertNull(confirmation.forCurrentNote("note-2").confirmableNoteId("note-2", "note-2"))
    }

    @Test
    fun losingWriteAccessRetainsTheOrdinaryEditDraft() {
        val draft = ChannelNoteDraftState(
            title = "edited title",
            content = "edited body",
            editing = true,
            taskDraft = null,
        )

        val afterAccessLoss = draft.onWriteAccessLost(savedContent = "saved body")

        assertEquals(draft, afterAccessLoss)
    }

    @Test
    fun losingWriteAccessKeepsTheActiveEditorAndTaskDrafts() {
        val draft = ChannelNoteDraftState(
            title = "edited title",
            content = "edited body",
            editing = true,
            taskDraft = "failed task body",
        )

        val afterAccessLoss = draft.onWriteAccessLost(savedContent = "saved body")

        assertEquals("edited title", afterAccessLoss.title)
        assertEquals("edited body", afterAccessLoss.content)
        assertTrue(afterAccessLoss.editing)
        assertEquals("failed task body", afterAccessLoss.taskDraft)
    }

    @Test
    fun losingWriteAccessKeepsTaskRetryAndRestoresSavedViewContent() {
        val draft = ChannelNoteDraftState(
            title = "saved title",
            content = "old body",
            editing = false,
            taskDraft = "unsaved checkbox body",
        )

        val afterAccessLoss = draft.onWriteAccessLost(savedContent = "saved body")

        assertEquals("unsaved checkbox body", afterAccessLoss.taskDraft)
        assertEquals("saved body", afterAccessLoss.content)
    }

    @Test
    fun saveGuardRejectsWritesAfterWriteAccessIsLost() {
        assertFalse(canPersistChannelNote(canWrite = false, busy = false))
        assertFalse(canPersistChannelNote(canWrite = true, busy = true))
        assertTrue(canPersistChannelNote(canWrite = true, busy = false))
    }

    @Test
    fun choosingLatestClearsTheTaskRetryDraft() {
        val draft = ChannelNoteDraftState(
            title = "edited title",
            content = "old body",
            editing = true,
            taskDraft = "failed task body",
        )

        val resolved = draft.useLatest(title = "latest title", content = "latest body")

        assertEquals("latest title", resolved.title)
        assertEquals("latest body", resolved.content)
        assertFalse(resolved.editing)
        assertNull(resolved.taskDraft)
    }

    @Test
    fun taskChangesDuringSaveAreQueuedWithoutAllowingUnauthorizedWrites() {
        assertEquals(
            ChannelNoteSaveAction.QueueTaskDraft,
            channelNoteSaveAction(canWrite = true, busy = true, hasTaskContent = true),
        )
        assertEquals(
            ChannelNoteSaveAction.Reject,
            channelNoteSaveAction(canWrite = true, busy = true, hasTaskContent = false),
        )
        assertEquals(
            ChannelNoteSaveAction.Reject,
            channelNoteSaveAction(canWrite = false, busy = true, hasTaskContent = true),
        )
        assertEquals(
            ChannelNoteSaveAction.Persist,
            channelNoteSaveAction(canWrite = true, busy = false, hasTaskContent = true),
        )
    }

    @Test
    fun lateCreateResponseCannotNavigateAwayFromANewerNoteRoute() {
        val route = ChannelNoteRouteGeneration("channel-1", null)
        val createGeneration = route.generation

        route.observe("channel-1", "note-1")

        assertFalse(route.matches("channel-1", null, createGeneration))
        assertTrue(route.matches("channel-1", "note-1", route.generation))
    }

    @Test
    fun lateDeleteResponseCannotNavigateAwayFromANewerNoteRoute() {
        val route = ChannelNoteRouteGeneration("channel-1", "note-1")
        val deleteGeneration = route.generation

        route.observe("channel-1", "note-2")

        assertFalse(route.matches("channel-1", "note-1", deleteGeneration))
        assertTrue(route.matches("channel-1", "note-2", route.generation))
    }

    @Test
    fun composeNoteTitleAndContentLengthsMatchTheWebContract() {
        assertEquals(200, CHANNEL_NOTE_TITLE_MAX_LENGTH)
        assertEquals(100_000, CHANNEL_NOTE_CONTENT_MAX_LENGTH)
    }

    @Test
    fun pendingTabSwitchCannotConfirmAfterBrowserHistoryChangesTheNote() {
        val route = ChannelNoteRouteGeneration("channel-1", "note-1")
        val pending = ChannelNoteTabSwitchState.pending("note-1", route.generation, "note-2")
        assertTrue(pending.matches("note-1", route.generation))

        route.observe("channel-1", "note-2")

        assertFalse(pending.matches("note-2", route.generation))
        assertEquals(
            ChannelNoteTabSwitchState.Empty,
            pending.forCurrentRoute("note-2", route.generation),
        )
        assertFalse(pending.matches("note-1", route.generation))
    }

    @Test
    fun latestNoteResponseCannotApplyToANewerRouteSelection() {
        val route = ChannelNoteRouteGeneration("channel-1", "note-1")
        val requestGeneration = route.generation

        route.observe("channel-1", "note-2")

        assertFalse(
            canApplyChannelNoteResponse(
                route,
                channelId = "channel-1",
                requestNoteId = "note-1",
                requestGeneration = requestGeneration,
                selectedNoteId = "note-2",
            ),
        )
        assertTrue(
            canApplyChannelNoteResponse(
                route,
                channelId = "channel-1",
                requestNoteId = "note-2",
                requestGeneration = route.generation,
                selectedNoteId = "note-2",
            ),
        )
    }

    @Test
    fun normalSaveConflictReadCannotApplyAfterTheRouteChanges() {
        val route = ChannelNoteRouteGeneration("channel-1", "note-a")
        val saveGeneration = route.generation

        route.observe("channel-1", "note-b")

        assertFalse(
            canApplyChannelNoteResponse(
                route,
                channelId = "channel-1",
                requestNoteId = "note-a",
                requestGeneration = saveGeneration,
                selectedNoteId = "note-b",
            ),
        )
    }
}
