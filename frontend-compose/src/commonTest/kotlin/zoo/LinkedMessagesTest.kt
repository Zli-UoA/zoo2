package zoo

import electric4k.RuntimeVisibilityAdapter
import electric4k.ShapeStreamOptions
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.request.HttpRequestData
import io.ktor.http.Headers
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest

private class FakeSource : LinkedChannelSource {
    override val state = MutableStateFlow(LinkedChannelSource.State())
    val requests = mutableListOf<String>()
    var closed = false
    override fun request(messageId: String) { requests += messageId }
    override fun close() { closed = true }
    fun deliver(row: Row) = state.update { it.copy(rows = it.rows + (row.text("id") to row), fetched = it.fetched + row.text("id")) }
}

@OptIn(ExperimentalCoroutinesApi::class)
class LinkedMessagesTest {
    private val message = obj("id" to "m1", "content" to "本文")
    private val sources = mutableListOf<FakeSource>()
    private val open = mutableMapOf<String, MutableStateFlow<List<Row>>>()

    private fun TestScope.loader() = LinkedMessages(
        backgroundScope, openRows = { open[it] }, openSource = { FakeSource().also(sources::add) },
        stallAfter = 10.seconds, releaseAfter = 30.seconds, clock = testScheduler.timeSource,
    )

    private class Observer(val job: Job, val states: MutableList<LinkedMessageState>) {
        val last get() = states.last()
    }

    private fun TestScope.observe(loader: LinkedMessages, channel: String = "c1", id: String = "m1"): Observer {
        val states = mutableListOf<LinkedMessageState>()
        val job = backgroundScope.launch { loader.observe(channel, id).collect { states += it } }
        runCurrent()
        return Observer(job, states)
    }

    @Test
    fun linksToTheSameChannelShareOneSubscription() = runTest {
        val loader = loader()
        val first = observe(loader)
        val second = observe(loader)
        observe(loader, id = "m2")
        observe(loader, channel = "c2")
        assertEquals(2, sources.size)
        sources.first().deliver(message)
        runCurrent()
        assertEquals(LinkedRow.Found(message), first.last.row)
        assertEquals(LinkedRow.Found(message), second.last.row)
    }

    @Test
    fun rowsOfAnOpenChannelAreUsedWithoutFetching() = runTest {
        val rows = MutableStateFlow(listOf(message, obj("id" to "m9", "ciphertext" to "{}")))
        open["c1"] = rows
        val loader = loader()
        assertEquals(LinkedRow.Found(message), loader.peek("c1", "m1").row)
        val observer = observe(loader)
        assertEquals(LinkedRow.Found(message), observer.last.row)
        assertTrue(observer.last.hasCiphertext)
        assertTrue(sources.isEmpty())

        // 開いているチャンネルの行から消えたら（取り直しなど）、1 件を取る。
        rows.value = emptyList()
        runCurrent()
        assertEquals(listOf("m1"), sources.single().requests)
    }

    @Test
    fun missingFailedAndStalledAreDistinct() = runTest {
        val loader = loader()
        val observer = observe(loader)
        assertEquals(LinkedMessageState(), observer.last)
        advanceTimeBy(9_999.milliseconds)
        runCurrent()
        assertFalse(observer.last.stalled)
        advanceTimeBy(2.milliseconds)
        runCurrent()
        assertTrue(observer.last.stalled)
        // あとで取得できれば、内容に切り替わる。
        sources.single().deliver(message)
        runCurrent()
        assertEquals(LinkedMessageState(LinkedRow.Found(message)), observer.last)

        val missing = observe(loader, id = "m2")
        sources.single().state.update { it.copy(fetched = it.fetched + "m2") }
        runCurrent()
        assertEquals(LinkedRow.Missing, missing.last.row)

        val failed = observe(loader, id = "m3")
        sources.single().state.update { it.copy(failed = setOf("m3")) }
        runCurrent()
        assertEquals(LinkedRow.Failed, failed.last.row)
    }

    @Test
    fun timeKeepsCountingWhileTheRowIsOffScreen() = runTest {
        val loader = loader()
        observe(loader).job.cancel()
        advanceTimeBy(11.seconds)
        runCurrent()
        assertTrue(observe(loader).states.first().stalled)
    }

    @Test
    fun theSubscriptionClosesAfterTheLastRowLeaves() = runTest {
        val loader = loader()
        val first = observe(loader)
        sources.single().deliver(message)
        runCurrent()
        first.job.cancel()
        advanceTimeBy(29.seconds)
        runCurrent()
        val back = observe(loader)
        assertEquals(1, sources.size)
        back.job.cancel()
        advanceTimeBy(31.seconds)
        runCurrent()
        assertTrue(sources.single().closed)

        // 閉じたあとは作り直し、取得が終わるまで前の結果を出す。取り直しに失敗しても残す。
        assertEquals(LinkedRow.Found(message), loader.peek("c1", "m1").row)
        val again = observe(loader)
        assertEquals(2, sources.size)
        assertEquals(LinkedRow.Found(message), again.states.first().row)
        sources.last().state.update { it.copy(failure = IllegalStateException("403")) }
        runCurrent()
        assertEquals(LinkedRow.Found(message), again.last.row)
    }

    @Test
    fun theNumberOfSubscribedChannelsIsLimited() = runTest {
        val loader = LinkedMessages(
            backgroundScope, openRows = { open[it] }, openSource = { FakeSource().also(sources::add) },
            clock = testScheduler.timeSource, maxChannels = 2,
        )
        val first = observe(loader, channel = "c1")
        observe(loader, channel = "c2")
        // 超えた分は取得せず、読み込めなかったものとする。
        val third = observe(loader, channel = "c3")
        assertEquals(2, sources.size)
        assertEquals(LinkedRow.Failed, third.last.row)
        // 使う行がなくなった購読があれば、閉じて空ける。
        first.job.cancel()
        runCurrent()
        assertEquals(3, sources.size)
        assertTrue(sources.first().closed)
        assertEquals(LinkedRow.Loading, third.last.row)
        sources.last().deliver(message)
        runCurrent()
        assertEquals(LinkedRow.Found(message), third.last.row)
    }

    @Test
    fun recordsThatDoNotLoadInTimeAreUnreadable() = runTest {
        val result = CompletableDeferred<Boolean>()
        var loaded = false
        var loads = 0
        val records = EncryptionRecordsLoader(
            backgroundScope, { loaded }, { loads++; result.await() }, 10.seconds, testScheduler.timeSource,
        )
        val states = mutableListOf<EncryptionRecords>()
        val first = backgroundScope.launch { records.states().collect { states += it } }
        runCurrent()
        assertEquals(listOf(EncryptionRecords.Loading), states)
        advanceTimeBy(10_001.milliseconds)
        runCurrent()
        assertEquals(EncryptionRecords.Unreadable, states.last())
        first.cancel()
        // 画面に戻っても読み込み中の枠に戻さず、読み込みは 1 回のまま。読めたら切り替える。
        assertEquals(EncryptionRecords.Unreadable, records.current())
        val again = mutableListOf<EncryptionRecords>()
        backgroundScope.launch { records.states().collect { again += it } }
        runCurrent()
        assertEquals(listOf(EncryptionRecords.Unreadable), again)
        loaded = true
        result.complete(true)
        runCurrent()
        assertEquals(EncryptionRecords.Loaded, again.last())
        assertEquals(EncryptionRecords.Loaded, records.current())
        assertEquals(1, loads)
    }

    @Test
    fun closingTheSessionClosesEverySubscription() = runTest {
        val loader = loader()
        observe(loader)
        loader.close()
        assertTrue(sources.single().closed)
    }
}

private class LinkPlatform : Platform {
    override suspend fun headers() = mapOf("Authorization" to "Bearer token")
    override suspend fun readCache(key: String): String? = error("linked messages are never read from disk")
    override suspend fun writeCache(key: String, value: String) = error("linked messages are never cached on disk")
    override fun uuid() = "new-id"
    override fun now() = "2026-11-26T00:00:00Z"
    override fun navigate(path: String) {}
    override fun translate(key: String, args: Map<String, String>) = key
    override fun action(name: String) {}
    override fun time(timestamp: String) = timestamp
    override fun day(timestamp: String) = timestamp
}

class LinkedChannelTableTest {
    private val config = Config("https://zoo.test", "/api/shapes", "alice", false)
    private val options = ShapeStreamOptions(
        liveSse = true,
        detectSystemWake = false,
        runtimeVisibility = object : RuntimeVisibilityAdapter {
            override fun isVisible() = true
            override fun subscribe(callback: (Boolean) -> Unit): () -> Unit = {}
        },
    )
    private val headers = headersOf(
        "electric-handle" to listOf("handle"),
        "electric-offset" to listOf("0_0"),
        "electric-schema" to listOf("""{"id":{"type":"text"},"content":{"type":"text"}}"""),
        "electric-cursor" to listOf("1"),
    )

    private fun snapshot(id: String) =
        """{"metadata":{"xmin":"1","xmax":"2","xip_list":[],"snapshot_mark":1,"database_lsn":"0"},""" +
            """"data":[{"key":"$id","value":{"id":"$id","content":"本文 $id"},"headers":{"operation":"insert"}}]}"""

    @Test
    fun onlyTheLinkedMessagesAreFetchedOncePerId() = runTest {
        val requests = mutableListOf<HttpRequestData>()
        val http = HttpClient(MockEngine { request ->
            requests += request
            val parameters = request.url.parameters
            val subset = parameters["subset__params"]
            when {
                subset != null -> respond(snapshot(if ("m2" in subset) "m2" else "m1"), headers = headers)
                parameters["live"] == "true" -> awaitCancellation()
                else -> respond("""[{"headers":{"control":"up-to-date"}}]""", headers = headers)
            }
        })
        val table = LinkedChannelTable("c1", config, LinkPlatform(), http, backgroundScope, options)
        try {
            table.request("m1")
            table.request("m1")
            table.request("m2")
            val state = table.state.first { it.fetched.size == 2 }
            assertEquals("本文 m1", state.rows.getValue("m1").text("content"))
            val subsets = requests.filter { it.url.parameters["subset__where"] != null }
            assertEquals(2, subsets.size)
            assertEquals(
                setOf("""{"1":"m1"}""", """{"1":"m2"}"""),
                subsets.map { it.url.parameters["subset__params"] }.toSet(),
            )
            requests.forEach {
                assertEquals("/api/shapes/messages", it.url.encodedPath)
                assertEquals("c1", it.url.parameters["channel_id"])
                assertEquals("changes_only", it.url.parameters["log"])
                // チャンネルの全件（offset=-1）は同期しない。
                assertTrue(it.url.parameters["offset"] != "-1", it.url.toString())
                assertEquals("Bearer token", it.headers["Authorization"])
            }
            assertEquals("id = $1", subsets.first().url.parameters["subset__where"])
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun onlyRequestedRowsAreKept() = runTest {
        var fetched = false
        var cursor = 1
        val http = HttpClient(MockEngine { request ->
            val parameters = request.url.parameters
            // 実際の Electric と同じく cursor を進める。同じままだと、取得のあとの up-to-date が抑止される。
            val next = Headers.build {
                appendAll(headers)
                set("electric-cursor", "${cursor++}")
            }
            when {
                parameters["subset__params"] != null -> {
                    fetched = true
                    respond(snapshot("m1"), headers = next)
                }
                // 取得したあとの live の読み取りは、取っていない行の追加と、取った行の編集を返す。取得の間は
                // live の要求が打ち切られるので、何度でも返す。
                parameters["live"] == "true" && fetched ->
                    respond(
                        """[{"key":"other","value":{"id":"other","content":"別の投稿"},"headers":{"operation":"insert"}},""" +
                            """{"key":"m1","value":{"id":"m1","content":"直した本文"},"headers":{"operation":"update"}},""" +
                            """{"headers":{"control":"up-to-date"}}]""",
                        headers = next,
                    )
                parameters["live"] == "true" -> awaitCancellation()
                else -> respond("""[{"headers":{"control":"up-to-date"}}]""", headers = next)
            }
        })
        // Electric4k は同じ Shape を作り直すと up-to-date の扱いが変わるので、テストごとに別のチャンネルにする。
        // live の待ち時間の上限は、テストの仮想の時刻ではすぐに過ぎて取り直しになるので、外す。
        val table = LinkedChannelTable(
            "kept", config, LinkPlatform(), http, backgroundScope,
            options.copy(liveSse = false, liveRequestTimeoutMillis = null),
        )
        try {
            table.request("m1")
            val state = table.state.first { it.rows["m1"]?.text("content") == "直した本文" }
            assertEquals(setOf("m1"), state.rows.keys)
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun aRejectedSubscriptionIsAFailure() = runTest {
        var requests = 0
        val http = HttpClient(MockEngine { requests++; respond("{}", HttpStatusCode.Forbidden) })
        val table = LinkedChannelTable("rejected", config, LinkPlatform(), http, backgroundScope, options)
        try {
            table.request("m1")
            table.state.first { it.failure != null }
            assertEquals(1, requests)
        } finally {
            table.close()
            http.close()
        }
    }
}
