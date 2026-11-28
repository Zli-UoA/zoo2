package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import kotlin.test.*

@OptIn(ExperimentalCoroutinesApi::class)
class HomeSubmissionTest {
    private class Files : Platform {
        val released = mutableListOf<String>()
        override suspend fun headers() = emptyMap<String, String>()
        override suspend fun readCache(key: String): String? = null
        override suspend fun writeCache(key: String, value: String) {}
        override fun uuid() = "submission"
        override fun now() = "2026-11-15T00:00:00Z"
        override fun navigate(path: String) {}
        override fun action(name: String) { if (name.startsWith("fileRelease:")) released += name }
        override fun translate(key: String, args: Map<String, String>) = key
        override fun time(timestamp: String) = timestamp
        override fun day(timestamp: String) = timestamp
    }

    private class Fixture(scope: CoroutineScope) {
        val files = Files()
        val app = Session(
            Repository(Config("https://zoo.test", "/api/shapes", "alice", false), files,
                HttpClient(MockEngine { error("Unexpected request") }), scope, synchronize = false),
            scope, Environment(),
        )
        val editorJob = SupervisorJob(scope.coroutineContext[Job])
        val discarded = mutableListOf<String>()
        val images = AttachmentDrafts(app, "", CoroutineScope(scope.coroutineContext + editorJob)) { obj("id" to "uploaded") }
        val state = HomeComposerState().apply { content = "送信する本文" }
        init {
            images.enabled = true
            images.onDiscard = { discarded += it.text("id") }
            images.add("""{"origin":"pick","files":[{"id":"local","name":"notes.txt","type":"text/plain","size":"4"}]}""")
        }
        fun leave() { editorJob.cancel(); images.close() }
        fun close() { images.close(); editorJob.cancel(); app.close() }
    }

    @Test
    fun sharedImagesAreValidatedTogetherBeforeChangingExistingDraft() = runTest {
        val fixture = Fixture(backgroundScope)
        try {
            fixture.images.maximumBytes = 8L * 1024 * 1024
            val result = fixture.images.addShare("""{"origin":"pick","files":[{"id":"small","name":"a.jpg","type":"image/jpeg","size":"20"},{"id":"big","name":"b.jpg","type":"image/jpeg","size":"8388609"}]}""")
            assertEquals("b.jpg: attachments.tooLarge", result)
            assertEquals(listOf("fileRelease:small", "fileRelease:big"), fixture.files.released)
            assertEquals(listOf("notes.txt"), fixture.images.items.map { it.name })
        } finally { fixture.close() }
    }

    @Test
    fun leavingTheEditorDoesNotCancelOrDuplicateAnAcceptedSubmission() = runTest {
        val fixture = Fixture(backgroundScope)
        val response = CompletableDeferred<Unit>()
        var posts = 0
        try {
            runCurrent()
            assertEquals("uploaded", fixture.images.items.single().uploaded!!.text("id"))
            fixture.state.submit(backgroundScope, fixture.images) {
                posts++
                response.await()
                fixture.images.release(fixture.images.items.toList())
                fixture.images.items.clear()
                fixture.state.content = ""
            }
            runCurrent()
            fixture.leave()
            runCurrent()
            assertTrue(fixture.state.sending)
            assertTrue(fixture.discarded.isEmpty(), "The uploaded image must remain available to the server")
            assertTrue(fixture.files.released.isEmpty())
            fixture.state.submit(backgroundScope, fixture.images) { posts++ }
            response.complete(Unit)
            runCurrent()
            assertEquals(1, posts)
            assertFalse(fixture.state.sending)
            assertFalse(fixture.state.failed)
            assertEquals("", fixture.state.content)
            assertTrue(fixture.discarded.isEmpty(), "A posted attachment must never be deleted as abandoned")
            assertEquals(listOf("fileRelease:local"), fixture.files.released)
        } finally { fixture.close() }
    }

    @Test
    fun failedSubmissionRetainsTheTextAndDiscardsDetachedAttachmentsOnlyAfterTheResponse() = runTest {
        val fixture = Fixture(backgroundScope)
        val response = CompletableDeferred<Unit>()
        try {
            runCurrent()
            fixture.state.submit(backgroundScope, fixture.images) {
                response.await()
                error("Server rejected the post")
            }
            runCurrent()
            fixture.leave()
            assertTrue(fixture.discarded.isEmpty())
            response.complete(Unit)
            runCurrent()
            assertFalse(fixture.state.sending)
            assertTrue(fixture.state.failed)
            assertEquals("送信する本文", fixture.state.content)
            assertEquals(listOf("uploaded"), fixture.discarded)
            assertEquals(listOf("fileRelease:local"), fixture.files.released)
            fixture.images.close()
            assertEquals(1, fixture.discarded.size)
        } finally { fixture.close() }
    }

    @Test
    fun sessionCancellationBeforeTheSendStartsReleasesOwnership() = runTest {
        val fixture = Fixture(backgroundScope)
        val sessionJob = SupervisorJob(backgroundScope.coroutineContext[Job])
        val sessionScope = CoroutineScope(backgroundScope.coroutineContext + sessionJob)
        try {
            runCurrent()
            fixture.state.submit(sessionScope, fixture.images) { fail("The cancelled request must not start") }
            fixture.leave()
            sessionJob.cancel()
            runCurrent()
            assertFalse(fixture.state.sending)
            assertEquals(listOf("uploaded"), fixture.discarded)
        } finally { sessionJob.cancel(); fixture.close() }
    }
}
