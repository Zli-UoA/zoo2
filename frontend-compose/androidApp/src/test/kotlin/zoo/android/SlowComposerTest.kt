package zoo.android

import android.app.Activity
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import io.ktor.http.content.TextContent
import kotlin.test.*
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
            {}, { emptyList() }, RecentPhotos(activity), {}, { true }, { _, _ -> })
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
            assertEquals(listOf("一件目", "二件目"), state.withOutgoing("/home", emptyList()).map { it.body })
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
