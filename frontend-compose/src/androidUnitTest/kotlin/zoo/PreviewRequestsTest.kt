package zoo

import java.util.concurrent.atomic.AtomicBoolean
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import okhttp3.Call
import okhttp3.OkHttpClient
import okhttp3.Request

class PreviewRequestsTest {
    @Test fun disposalBetweenTheInitialCheckAndRegistrationCancelsTheCall() {
        val disposed = AtomicBoolean(false)
        val calls = object : LinkedHashSet<Call>() {
            override fun add(element: Call): Boolean {
                // The viewer finishes closing just as the Call joins its cancellation set.
                disposed.set(true)
                return super.add(element)
            }
        }
        val call = OkHttpClient().newCall(Request.Builder().url("https://example.test/").build())
        assertFalse(registerPreviewCall(call, calls, disposed))
        assertTrue(call.isCanceled())
        assertTrue(calls.isEmpty())
    }
    @Test fun openViewersRetainTheirCallForDisposal() {
        val calls = mutableSetOf<Call>()
        val call = OkHttpClient().newCall(Request.Builder().url("https://example.test/").build())
        assertTrue(registerPreviewCall(call, calls, AtomicBoolean(false)))
        assertTrue(call in calls)
        assertFalse(call.isCanceled())
        call.cancel()
    }
}
