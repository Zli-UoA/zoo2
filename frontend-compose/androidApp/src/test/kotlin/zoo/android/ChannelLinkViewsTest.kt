package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import zoo.ChannelLinkInfo
import zoo.LinkedChannel
import zoo.LinkedMessage
import zoo.LinkedMessageKind
import zoo.android.ui.ChannelLinkBody
import zoo.android.ui.ChannelLinkView
import zoo.android.ui.UiIcon

class ChannelLinkViewsTest {
    private val channel = LinkedChannel("c", "開発", "雑談", isPrivate = false, isDm = false, encrypted = false)
    private val message = LinkedMessage(
        "m", null, "はる", "2026-11-23 07:30:00", LinkedMessageKind.Text, "こんにちは", 0, forwarded = false,
    )

    private fun view(info: ChannelLinkInfo) = channelLinkView(
        info, { key, args -> (listOf(key) + args.map { "${it.first}=${it.second}" }).joinToString("|") }, { "11/23 16:30" },
    )

    private fun body(message: LinkedMessage) =
        (view(ChannelLinkInfo.Message(channel, message)) as ChannelLinkView.Message).body

    @Test
    fun statesUseTheSharedWording() {
        assertEquals(ChannelLinkView.Loading("channelLink.loading"), view(ChannelLinkInfo.Loading))
        assertEquals(
            ChannelLinkView.Notice(UiIcon.CloudOff, "channelLink.loadFailed"), view(ChannelLinkInfo.Failed),
        )
        assertEquals(
            ChannelLinkView.Notice(UiIcon.VisibilityOff, "channelLink.messageUnavailable"),
            view(ChannelLinkInfo.Unavailable(message = true)),
        )
        assertEquals(
            ChannelLinkView.Notice(UiIcon.VisibilityOff, "channelLink.channelUnavailable"),
            view(ChannelLinkInfo.Unavailable(message = false)),
        )
    }

    @Test
    fun channelsNameDirectMessagesGenerically() {
        assertEquals(ChannelLinkView.Channel("#開発", null, "雑談"), view(ChannelLinkInfo.Channel(channel)))
        val dm = view(ChannelLinkInfo.Channel(channel.copy(name = "dm:a:b", isDm = true, isPrivate = true)))
        assertEquals(ChannelLinkView.Channel("dms.title", null, "雑談"), dm)
        assertEquals(
            "channelLink.privateChannel",
            (view(ChannelLinkInfo.Channel(channel.copy(isPrivate = true))) as ChannelLinkView.Channel).note,
        )
        assertEquals(
            "channelLink.encryptedChannel",
            (view(ChannelLinkInfo.Channel(channel.copy(isPrivate = true, encrypted = true))) as ChannelLinkView.Channel).note,
        )
    }

    @Test
    fun messagesCarryTheShortLabelAndThread() {
        val shown = view(ChannelLinkInfo.Message(channel, message.copy(parentId = "p"))) as ChannelLinkView.Message
        assertEquals("channelLink.inlineMessage|name=はる|channel=#開発", shown.inlineLabel)
        assertEquals("channelLink.thread|channel=#開発", shown.channel)
        assertEquals("p", shown.parentId)
        assertEquals("11/23 16:30", shown.time)
        assertEquals("#開発", (view(ChannelLinkInfo.Message(channel, message)) as ChannelLinkView.Message).channel)
    }

    @Test
    fun theBodyFollowsTheKindOfMessage() {
        assertEquals(ChannelLinkBody("こんにちは"), body(message))
        assertEquals(
            ChannelLinkBody("channelLink.encrypted", UiIcon.Lock, italic = true),
            body(message.copy(kind = LinkedMessageKind.Encrypted, excerpt = "")),
        )
        assertEquals(
            ChannelLinkBody("channelLink.appMessage", UiIcon.Widgets),
            body(message.copy(kind = LinkedMessageKind.App, excerpt = "")),
        )
        assertEquals(
            ChannelLinkBody("notifications.forwardedPreview", UiIcon.Forward),
            body(message.copy(excerpt = "", forwarded = true, attachmentCount = 1)),
        )
        assertEquals(
            ChannelLinkBody("channelLink.attachments|count=2", UiIcon.Attach),
            body(message.copy(excerpt = "", attachmentCount = 2)),
        )
        assertNull(body(message.copy(excerpt = "")))
    }

    @Test
    fun theYearIsShownOnlyForOtherYears() {
        val day: (String) -> String = { it.substringBefore(' ') }
        val time: (String) -> String = { it.substringAfter(' ') }
        assertEquals("11/23 16:30", channelLinkTime("2026/11/23 16:30", "2026/11/26 00:00", day, time))
        assertEquals("2025/11/23 16:30", channelLinkTime("2025/11/23 16:30", "2026/11/26 00:00", day, time))
    }
}
