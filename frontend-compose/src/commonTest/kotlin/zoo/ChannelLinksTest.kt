package zoo

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

class ChannelLinksTest {
    private val app = "https://zoo.test"
    private val message = ChannelLinkTarget("general", messageId = "m1")
    private val channelRow = obj("id" to "general", "channel_name" to "general", "description" to "雑談")
    private val channel = linkedChannel(listOf(channelRow), "general")!!
    private val users = mapOf("alice" to obj("id" to "alice", "display_name" to "アリス"))

    private fun row(vararg extra: Pair<String, Any?>) = obj(
        "id" to "m1", "channel_id" to "general", "user_id" to "alice", "content" to "**こんにちは**",
        "created_at" to "2026-11-23 07:30:00", *extra,
    )

    private fun resolve(
        row: LinkedRow = LinkedRow.Found(row()),
        target: ChannelLinkTarget = message,
        channel: LinkedChannel? = this.channel,
        ready: Boolean = true,
        recorded: Boolean = false,
        ciphertext: Boolean = false,
        stalled: Boolean = false,
        records: EncryptionRecords = EncryptionRecords.Loaded,
        hidesContent: Boolean = false,
    ) = resolveChannelLink(
        target, channel, ready, LinkedMessageState(row, ciphertext, stalled), recorded, records, users,
    ) { hidesContent }

    private fun shown(info: ChannelLinkInfo) = assertIs<ChannelLinkInfo.Message>(info).message

    @Test
    fun onlyThisAppsChannelPagesAreInAppLinks() {
        assertEquals(ChannelLinkTarget("c1"), parseChannelLink("https://zoo.test/channels/c1", app))
        assertEquals(ChannelLinkTarget("c1"), parseChannelLink("https://ZOO.test:443/channels/c1/", "$app/"))
        assertEquals(
            ChannelLinkTarget("c1", messageId = "m2", threadId = "m1"),
            parseChannelLink("https://zoo.test/channels/c1?thread=m1&message=m2&x=1", app),
        )
        assertEquals("a b+c", parseChannelLink("https://zoo.test/channels/a%20b+c", app)?.channelId)
        assertNull(parseChannelLink("https://zoo.test/channels/c1?message=", app)?.messageId)
        for (other in listOf(
            "http://zoo.test/channels/c1", "https://zoo.test:8443/channels/c1", "https://zoo.test.evil/channels/c1",
            "https://evil@zoo.test/channels/c1", "https://other.test/channels/c1", "https://zoo.test/channels",
            "https://zoo.test/channels/c1/settings", "https://zoo.test/api/channels/c1", "https://zoo.test/",
            "ftp://zoo.test/channels/c1", "/channels/c1",
        )) assertNull(parseChannelLink(other, app), other)
    }

    @Test
    fun threadOnlyLinksPointAtTheParent() {
        assertEquals("m1", ChannelLinkTarget("c", threadId = "m1").linkedMessageId)
        assertEquals("m2", ChannelLinkTarget("c", messageId = "m2", threadId = "m1").linkedMessageId)
        assertNull(ChannelLinkTarget("c").linkedMessageId)
    }

    @Test
    fun destinationKeepsTheMessageFirstAndNamesTheThread() {
        assertEquals("/channels/c?open=7", channelLinkDestination(ChannelLinkTarget("c"), null, 7))
        assertEquals("/channels/c?message=m&open=7", channelLinkDestination(ChannelLinkTarget("c", "m"), null, 7))
        val reply = channelLinkDestination(ChannelLinkTarget("c", "m", "p"), "p", 7)
        assertEquals("/channels/c?message=m&thread=p&open=7", reply)
        assertEquals("p", linkedThread(reply))
        assertNull(linkedThread("/channels/c?message=m"))
        assertNull(linkedThread("/search?thread=p"))
    }

    @Test
    fun ownLinksAreEmbeddedAndOnlyOtherUrlsGoToTheExternalCard() {
        val links = messageLinks(
            "$app/channels/c?message=m `$app/channels/code` https://example.com/a https://example.com/b", app,
        )
        assertEquals(listOf("$app/channels/c?message=m"), links.channels.map { it.first })
        assertEquals("https://example.com/a", links.external)
        // 合わせて 3 件まで（Web と同じ）。
        val many = messageLinks((1..5).joinToString(" ") { "$app/channels/c$it" }, app)
        assertEquals(3, many.channels.size)
        assertNull(many.external)
        assertNull(messageLinks("?[静か]($app/channels/c)", app).channels.firstOrNull())
        // 添付として出す画像の URL は、外部のカードにしない。
        assertEquals(
            "https://example.com/b",
            messageLinks(
                "https://img.test/a.png https://example.com/b", app, attached = setOf("https://img.test/a.png"),
            ).external,
        )
    }

    @Test
    fun channelsOutsideTheListAreUnavailableOnlyAfterTheListSynced() {
        assertEquals(ChannelLinkInfo.Loading, resolve(channel = null, ready = false))
        assertEquals(ChannelLinkInfo.Unavailable(message = true), resolve(channel = null))
        assertEquals(
            ChannelLinkInfo.Unavailable(message = false),
            resolve(channel = null, target = ChannelLinkTarget("general")),
        )
        assertNull(linkedChannel(listOf(obj("id" to "general", "deleted_at" to "2026-01-01")), "general"))
        assertNull(linkedChannel(listOf(channelRow), "private"))
    }

    @Test
    fun fetchStatesAreDistinguishedFromUnavailable() {
        assertEquals(ChannelLinkInfo.Loading, resolve(LinkedRow.Loading))
        assertEquals(ChannelLinkInfo.Failed, resolve(LinkedRow.Loading, stalled = true))
        assertEquals(ChannelLinkInfo.Failed, resolve(LinkedRow.Failed))
        assertEquals(ChannelLinkInfo.Unavailable(true), resolve(LinkedRow.Missing))
        assertEquals(ChannelLinkInfo.Unavailable(true), resolve(LinkedRow.Found(row("deleted_at" to "2026-11-24"))))
        assertEquals(ChannelLinkInfo.Unavailable(true), resolve(LinkedRow.Found(row("id" to "other"))))
    }

    @Test
    fun channelLinksNeedNoMessage() {
        val info = resolve(LinkedRow.Loading, target = ChannelLinkTarget("general"), records = EncryptionRecords.Loading)
        assertEquals(ChannelLinkInfo.Channel(channel), info)
        val recorded = resolve(LinkedRow.Loading, target = ChannelLinkTarget("general"), recorded = true)
        assertTrue(assertIs<ChannelLinkInfo.Channel>(recorded).channel.encrypted)
    }

    @Test
    fun messagesShowTheAuthorAndAPlainExcerpt() {
        val message = shown(resolve(LinkedRow.Found(row("parent_id" to "p"))))
        assertEquals("アリス", message.authorName)
        assertEquals("こんにちは", message.excerpt)
        assertEquals("p", message.parentId)
        assertEquals(LinkedMessageKind.Text, message.kind)
        assertEquals("bob", shown(resolve(LinkedRow.Found(row("user_id" to "bob")))).authorName)
        assertEquals(
            "Webhook", shown(resolve(LinkedRow.Found(row("display_name" to "Webhook")))).authorName,
        )
    }

    @Test
    fun encryptedChannelsNeverShowTheBody() {
        fun assertHidden(info: ChannelLinkInfo) {
            val message = shown(info)
            assertEquals(LinkedMessageKind.Encrypted, message.kind)
            assertEquals("", message.excerpt)
        }
        // サーバーの encrypted、端末の記録、edm: の名前、対象の行や同じチャンネルの暗号文。
        assertHidden(resolve(channel = channel.copy(encrypted = true)))
        assertHidden(resolve(recorded = true))
        assertHidden(resolve(channel = channel.copy(name = "edm:alice:bob")))
        assertHidden(resolve(ciphertext = true))
        assertHidden(resolve(LinkedRow.Found(row("ciphertext" to "{}"))))
        // 本文を出さない種類は、記録の読み込みを待たない。
        assertHidden(resolve(channel = channel.copy(encrypted = true), records = EncryptionRecords.Loading))
    }

    @Test
    fun plaintextWaitsForTheEncryptionRecords() {
        assertEquals(ChannelLinkInfo.Loading, resolve(records = EncryptionRecords.Loading))
        assertEquals(ChannelLinkInfo.Failed, resolve(records = EncryptionRecords.Unreadable))
    }

    @Test
    fun appPostsAndForwardsDoNotShowContent() {
        val app = shown(resolve(hidesContent = true))
        assertEquals(LinkedMessageKind.App, app.kind)
        assertEquals("", app.excerpt)
        val forward = shown(resolve(LinkedRow.Found(row(
            "content" to "", "forwarded_message_ids" to JsonArray(listOf(JsonPrimitive("x"))),
        ))))
        assertTrue(forward.forwarded)
        assertEquals("", forward.excerpt)
        val files = shown(resolve(LinkedRow.Found(row("content" to "", "display_data" to buildJsonObject {
            putJsonArray("attachments") { repeat(2) { add(buildJsonObject { put("id", "a$it") }) } }
        }))))
        assertEquals(2, files.attachmentCount)
    }

    @Test
    fun excerptsDropDecorationLinksAndCode() {
        assertEquals(
            "見出し 太字 リンク https://example.com コード 本文 引用 中央 ルビ",
            messageExcerpt(
                "# 見出し\n**太字** [リンク](https://example.com/x) https://example.com\n" +
                    "```kotlin\nコード\n```\n<plain>本文</plain>\n> 引用\n<center>中央</center> \$[ruby ルビ るび]",
            ),
        )
        assertEquals("", messageExcerpt("  \n "))
        assertEquals("a".repeat(200) + "…", messageExcerpt("a".repeat(300)))
        // サロゲートペアは割らない。
        assertEquals("😀".repeat(3) + "…", messageExcerpt("😀".repeat(5), maxLength = 3))
        assertEquals("<script>", messageExcerpt("<script>"))
    }

    @Test
    fun oneBodyFetchesOnlyAFewMessages() {
        fun target(n: Int) = ChannelLinkTarget("c$n", "m$n")
        assertEquals(
            setOf("c2" to "m2", "c1" to "m1", "c3" to "m3"),
            fetchedLinks(
                listOf(target(2), ChannelLinkTarget("only")),
                listOf(target(1), target(2), target(3), target(4)),
            ),
        )
    }

    @Test
    fun urlsEndWhereTheWebEndsThem() {
        val url = "https://zli.chat/channels/a?message=m"
        for (text in listOf(
            "${url}を見て", "詳しくは$url。", "「$url」", "（$url）", "($url).", "<$url>", "$url(未閉じ",
        )) {
            assertEquals(listOf(url), previewUrls(text), text)
            assertEquals(
                listOf(url), parseInline(text).mapNotNull { it.url }, text,
            )
        }
        // 末尾で除くのは `.` と `,` だけ（Web と同じ）。
        assertEquals(listOf("$url!"), previewUrls("$url! 次"))
        assertEquals(listOf(url), previewUrls("$url, 次"))
        // 閉じない括弧や開き括弧が並んでも、すぐに終わる。
        for (content in listOf("[".repeat(40_000), "https://a(".repeat(4_000), "[](https://a".repeat(3_000))) {
            previewUrls(content)
        }
        // 対応しない閉じ括弧があれば、閉じていない括弧より前で終わる。
        assertEquals(listOf("https://example.com/a"), previewUrls("https://example.com/a(b]"))
        // 装飾などの角括弧を含むラベルも、行き先だけを数える。サイレントリンクは数えない。
        // ホストだけの URL は、末尾に `/` を付けた形で返る。
        assertEquals(listOf("https://b.test/"), previewUrls("[\$[fg.color=ff0000 https://a.test]](https://b.test)"))
        assertEquals(emptyList(), previewUrls("?[a [b] c](https://silent.test)"))
        // `[文字](URL)` の行き先は、押して開くリンクと同じに切る。文字の中の URL は数えない。
        for (content in listOf(
            "[a](https://b.test/page.)",
            "[https://a.test](https://b.test/x;y)",
            "[a](https://b.test/日本)",
            "[\$[x2 https://a.test]](https://b.test/x;y)",
            "[https://a.test](https://b.test/x y)",
        )) {
            val opened = parseInline(content).mapNotNull { token -> token.url?.let(::publicUrlString) }
            assertEquals(opened, previewUrls(content), content)
        }
        // 文字に URL を書いたリンクは、URL そのものを書いたリンクと区別する。
        val labelled = parseInline("[https://a.test](https://a.test) https://c.test <https://d.test>")
        assertEquals(listOf(false, true, true), labelled.filter { it.url != null }.map { it.bare })
        assertEquals(
            listOf("https://zli.chat/channels/b"),
            previewUrls("[https://zli.chat/channels/a](https://zli.chat/channels/b)"),
        )
        assertEquals(
            listOf("https://a.test/wiki/A_(b)", "https://b.test/?u=https://c.test"),
            previewUrls("[x](https://a.test/wiki/A_(b)) https://b.test/?u=https://c.test"),
        )
    }
}
