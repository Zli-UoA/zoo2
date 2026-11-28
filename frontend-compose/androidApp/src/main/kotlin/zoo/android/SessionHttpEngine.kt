package zoo.android

import io.ktor.client.engine.HttpClientEngineBase
import io.ktor.client.engine.callContext
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.engine.okhttp.OkHttpConfig
import io.ktor.client.request.HttpRequestData
import io.ktor.client.request.HttpResponseData
import io.ktor.utils.io.InternalAPI
import kotlinx.coroutines.Job
import kotlinx.coroutines.ensureActive
import okhttp3.ConnectionPool

/** Selects a fresh connection pool for new calls after returning to the foreground. */
internal class SessionHttpEngine(
    private val configure: OkHttpConfig.() -> Unit,
) : HttpClientEngineBase("zoo-session") {
    private val lock = Any()
    private var current = createEngine()
    private var closed = false

    override val config = current.config
    override val supportedCapabilities = current.supportedCapabilities

    private fun createEngine() = OkHttp.create {
        configure()
        // Ktor may use several clients with different timeouts. They share this pool.
        val connections = ConnectionPool()
        config { connectionPool(connections) }
    }

    fun reconnect() {
        val retired = synchronized(lock) {
            if (closed) return
            val previous = current
            current = createEngine()
            previous
        }
        // OkHttp cleans up on its own coroutine once calls using the old engine finish.
        retired.close()
    }

    @OptIn(InternalAPI::class)
    override suspend fun execute(data: HttpRequestData): HttpResponseData {
        val call = callContext()
        call.ensureActive()
        val (engine, lease) = synchronized(lock) {
            check(!closed) { "Session HTTP engine is closed" }
            current to Job(current.coroutineContext[Job])
        }
        // Keep the engine alive through streaming response bodies as well as request headers.
        call[Job]!!.invokeOnCompletion { lease.complete() }
        return engine.execute(data)
    }

    override fun close() {
        val retired = synchronized(lock) {
            if (closed) return
            closed = true
            current
        }
        super.close()
        retired.close()
    }
}
