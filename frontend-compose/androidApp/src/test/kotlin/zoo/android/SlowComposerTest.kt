package zoo.android

import android.app.Activity
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import io.ktor.http.content.TextContent
import kotlin.test.*
import kotlinx.coroutines.launch
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import zoo.*
import zoo.android.phone.AndroidPhoneHost
import zoo.android.ui.*

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class SlowComposerTest {
    @Test fun delayedTypingStartIsFollowedByStopForTheSameThreadWithoutDraftText() = runTest {
        val first = CompletableDeferred<Unit>()
        val updates = mutableListOf<JsonObject>()
        val reads = mutableListOf<String?>()
        val http = HttpClient(MockEngine(MockEngineConfig().apply {
            dispatcher = StandardTestDispatcher(testScheduler)
            addHandler { request ->
                assertEquals("/api/channels/room/typing", request.url.encodedPath)
                val result = if (request.method == HttpMethod.Post) {
                    updates += Json.parseToJsonElement((request.body as TextContent).text).jsonObject
                    if (updates.size == 1) first.await()
                    "{}"
                } else {
                    reads += request.url.parameters["parent_id"]
                    """[{"id":"other","display_name":"はる"}]"""
                }
                respond(result, headers = headersOf(HttpHeaders.ContentType, "application/json"))
            }
        }))
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val platform = AndroidPlatform(activity, http, DeviceAuth(http), "https://zoo.test", "me", {}, { emptyList() },
            RecentPhotos(activity), {}, { true }, { _, _, _ -> })
        val transport = object : Platform by platform {
            override suspend fun headers() = emptyMap<String, String>()
            override fun translate(key: String, args: Map<String, String>) = args["names"] ?: key
        }
        val config = zoo.Config("https://zoo.test", "/api/shapes", "me", false)
        val session = Session(Repository(config, transport, http, backgroundScope, synchronize = false), backgroundScope, platform.env)
        val presence = ChannelTyping(session, "room", "thread")
        val poll = backgroundScope.launch { presence.poll() }
        presence.edit(true)
        runCurrent()
        assertEquals("はる", presence.label)
        assertTrue(reads.isNotEmpty() && reads.all { it == "thread" })
        advanceTimeBy(3001)
        assertEquals(1, updates.size)
        first.complete(Unit)
        runCurrent()
        assertEquals(listOf(true, false), updates.map { it.flag("active") })
        assertEquals(1, updates.map { it.text("session_id") }.distinct().size)
        assertTrue(updates.all { it.keys == setOf("session_id", "parent_id", "active") && it.text("parent_id") == "thread" })
        poll.cancel()
        presence.close()
        http.close()
    }

    @Test fun subsequentHomeDraftsAreIndependentOfHeldAcknowledgements() = runTest {
        val held = CompletableDeferred<Unit>()
        val writes = mutableListOf<String>()
        val engine = MockEngine(MockEngineConfig().apply {
            dispatcher = StandardTestDispatcher(testScheduler)
            addHandler { request ->
                val response = if (request.method == HttpMethod.Post && request.url.encodedPath == "/api/home") {
                    writes += Json.parseToJsonElement((request.body as TextContent).text).jsonObject.text("content")
                    held.await()
                    """{"id":"post-${writes.size}"}"""
                } else "[]"
                respond(response, headers = headersOf(HttpHeaders.ContentType, "application/json"))
            }
        })
        val http = HttpClient(engine)
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val config = zoo.Config("https://zoo.test", "/api/shapes", "me", false)
        val platform = AndroidPlatform(activity, http, DeviceAuth(http), config.apiBaseUrl, "me",
            {}, { emptyList() }, RecentPhotos(activity), {}, { true }, { _, _, _ -> })
        val transport = object : Platform by platform {
            override suspend fun headers() = emptyMap<String, String>()
        }
        val session = Session(Repository(config, transport, http, backgroundScope, synchronize = false), backgroundScope, platform.env)
        val phone = AndroidPhoneHost(activity, session, http) {}
        val state = ConnectedScreenState(session, platform, phone.controller)
        try {
            val screen = AndroidScreen.NewPost(InputState(), emptyList(), false)
            for (text in listOf("一件目", "二件目")) {
                state.postDraft = text
                state.dispatch(AndroidUiAction.UseInputTool(InputTool.Send), screen, null, emptyList(), null, null) {}
                assertEquals("", state.postDraft)
            }
            state.postDraft = "三件目を編集中"
            runCurrent()
            assertEquals(listOf("一件目"), writes)
            val pending = state.withOutgoing("/home", emptyList())
            assertEquals(listOf("一件目", "二件目"), pending.map { it.body })
            assertTrue(pending.all {
                it.delivery == EntryDelivery.Sending && it.authorId == "me" && it.time.isNotBlank() && it.postHeader != null
            })
            held.complete(Unit)
            runCurrent()
            assertEquals(listOf("一件目", "二件目"), writes)
            assertEquals("三件目を編集中", state.postDraft)
        } finally {
            state.close()
            phone.close()
            http.close()
        }
    }
}
