package zoo.android

import android.content.Context
import java.io.IOException
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class PushSubscriptionRegistrationTest {
    private val context: Context get() = RuntimeEnvironment.getApplication()
    private val base = "https://zoo.example"
    private val user = "me"

    @BeforeTest fun reset() {
        for (name in listOf("push-binding", "push-subscription-cleanup"))
            context.getSharedPreferences(name, Context.MODE_PRIVATE).edit().clear().commit()
    }

    @Test fun failedLogoutCleanupDoesNotAccumulateTwentySubscriptions() = runTest {
        val server = mutableSetOf<String>()
        repeat(25) {
            val store = PushBinding(context) // The journal survives a process/store replacement.
            val binding = store.bind(base, user)
            val endpoint = "fcm:token:$binding"
            val registration = PushSubscriptionRegistration(store, base, user, binding) { method, id ->
                if (method == "DELETE") server.remove(id) else {
                    check(server.size < 20)
                    server += id
                }
            }
            registration.register(endpoint)
            assertEquals(setOf(endpoint), server)
            store.clear(binding)
            val offline = PushSubscriptionRegistration(store, base, user, binding) { _, _ -> throw IOException("offline") }
            assertFailsWith<IOException> { offline.cleanup() }
            assertEquals(listOf(endpoint), store.pendingRemovals(base, user))
        }
    }

    @Test fun lostPostResponseIsRetriedAsCleanupBeforeTheNextRegistration() = runTest {
        val store = PushBinding(context)
        val binding = store.bind(base, user)
        val old = "fcm:token:$binding"
        val server = mutableSetOf<String>()
        val uncertain = PushSubscriptionRegistration(store, base, user, binding) { _, endpoint ->
            server += endpoint
            throw IOException("lost response after commit")
        }
        assertFailsWith<IOException> { uncertain.register(old) }
        store.forgetAll()
        val fresh = PushBinding(context)
        val next = fresh.bind(base, user)
        val operations = mutableListOf<String>()
        PushSubscriptionRegistration(fresh, base, user, next) { method, endpoint ->
            operations += method
            if (method == "DELETE") server.remove(endpoint) else server.add(endpoint)
        }.register("fcm:token:$next")
        assertEquals(listOf("DELETE", "POST"), operations)
        assertEquals(setOf("fcm:token:$next"), server)
    }

    @Test fun cancelledInFlightPostIsDeletedAfterItsAcknowledgementIfLoggedOut() = runTest {
        val store = PushBinding(context)
        val binding = store.bind(base, user)
        val endpoint = "fcm:token:$binding"
        val acknowledged = CompletableDeferred<Unit>()
        val server = mutableSetOf<String>()
        val registration = PushSubscriptionRegistration(store, base, user, binding) { method, id ->
            if (method == "POST") { server += id; acknowledged.await() } else server.remove(id)
        }
        val job = launch { registration.register(endpoint) }
        runCurrent()
        assertEquals(setOf(endpoint), server)
        store.clear(binding)
        job.cancel()
        acknowledged.complete(Unit)
        job.join()
        assertTrue(server.isEmpty())
        assertTrue(store.pendingRemovals(base, user).isEmpty())
    }

    @Test fun pendingCleanupIsLimitedToTheAuthenticatedServerAndUser() = runTest {
        val store = PushBinding(context)
        store.deferRemoval(base, "other", "other-user")
        store.deferRemoval("https://other.example", user, "other-server")
        store.deferRemoval(base, user, "mine")
        val deleted = mutableListOf<String>()
        PushSubscriptionRegistration(store, base, user, "unused") { _, endpoint -> deleted += endpoint }.cleanup()
        assertEquals(listOf("mine"), deleted)
        assertEquals(listOf("other-user"), store.pendingRemovals(base, "other"))
        assertEquals(listOf("other-server"), store.pendingRemovals("https://other.example", user))
    }

    @Test fun rotatingATokenRemovesItsOldRegistrationBeforeTheDeviceLimitCheck() = runTest {
        val store = PushBinding(context)
        val binding = store.bind(base, user)
        store.registered(binding, "old-token")
        val server = (1..19).map { "other-$it" }.toMutableSet().apply { add("old-token") }
        PushSubscriptionRegistration(store, base, user, binding) { method, endpoint ->
            if (method == "DELETE") server.remove(endpoint) else {
                check(server.size < 20)
                server.add(endpoint)
            }
        }.register("new-token")
        assertEquals(20, server.size)
        assertTrue("new-token" in server)
        assertFalse("old-token" in server)
    }
}
