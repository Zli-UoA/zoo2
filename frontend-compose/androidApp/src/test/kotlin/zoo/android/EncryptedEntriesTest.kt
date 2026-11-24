package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue
import zoo.DecryptedBody
import zoo.MessageRevision
import zoo.RecipientNotice
import zoo.UndeliverableReason
import zoo.android.ui.TimelineItem
import zoo.obj

/** 暗号化チャンネルの行の表示（Web の `MessageRow`・`MessageList`・`EncryptionRecipientsNotice` と同じ規則）。 */
class EncryptedEntriesTest {
    private val t: (String) -> String = { it }
    private val base = TimelineItem("m", "alice", "", "12:00", editable = true, link = "https://example.com")
    private val encryptedRow = obj("id" to "m", "ciphertext" to "x", "epoch" to "1")
    private val plainRow = obj("id" to "p", "content" to "before")

    @Test
    fun undecryptableRowsShowTheReasonInsteadOfTheBody() {
        val entry = encryptedEntry(base, encryptedRow, DecryptedBody(notice = "e2ee.undecryptable.keyMissing"), true, 0, 0, t)
        assertEquals("e2ee.undecryptable.keyMissing", entry.notice)
        assertEquals("", entry.body)
        assertTrue(entry.marks.isEmpty())
        assertFalse(entry.editable)
    }

    @Test
    fun senderAndRevisionWarningsAreMarksNextToTheBody() {
        val entry = encryptedEntry(base, encryptedRow, DecryptedBody("hello", "e2ee.sender.unverified"), true, 0, 0, t)
        assertEquals("hello", entry.body)
        assertNull(entry.notice)
        assertEquals(listOf("e2ee.sender.unverified"), entry.marks)
        // 暗号化した本文の URL は自動で取得しない。
        assertNull(entry.link)
        assertTrue(entry.editable)
        val stale = encryptedEntry(base, encryptedRow, DecryptedBody(revision = MessageRevision.STALE), true, 0, 0, t)
        assertEquals(listOf("e2ee.revision.stale"), stale.marks)
        assertFalse(stale.editable)
    }

    @Test
    fun plaintextHistoryGetsADividerAndOnlyLatePlaintextIsMarked() {
        val before = encryptedEntry(base.copy(id = "p"), plainRow, null, true, 0, 1, t)
        assertTrue(before.marks.isEmpty())
        assertNull(before.divider)
        assertFalse(before.editable)
        val first = encryptedEntry(base, encryptedRow, DecryptedBody("hi"), true, 1, 1, t)
        assertEquals("e2ee.legacy.divider", first.divider)
        val late = encryptedEntry(base.copy(id = "late"), plainRow, null, true, 2, 1, t)
        assertEquals(listOf("e2ee.legacy.unencryptedBadge"), late.marks)
        // 平文チャンネルでは何も変えない。
        assertEquals(base, encryptedEntry(base, plainRow, null, false, 3, -1, t))
    }

    @Test
    fun encryptedAttachmentsComeFromTheDecryptedPayload() {
        val info = obj("attachment_id" to "a1", "name" to "photo.png", "mimetype" to "image/png", "size" to 3, "file" to obj("v" to "v2"))
        val entry = encryptedEntry(base.copy(attachments = listOf("server" to "encrypted.bin")), encryptedRow,
            DecryptedBody("x", attachments = listOf(info)), true, 0, 0, t)
        assertEquals(listOf("a1" to "photo.png"), entry.attachments)
        assertEquals(info, entry.encryptedFiles["a1"])
        assertEquals("application/octet-stream", safeAttachmentType(obj("mimetype" to "text/html")))
        assertEquals("image/png", safeAttachmentType(info))
    }

    @Test
    fun recipientNoticeShowsOnlyAddedAndUndeliverable() {
        val translate: (String, Array<out Pair<String, String>>) -> String = { key, args -> key + args.joinToString { "${it.first}=${it.second}" } }
        assertEquals(emptyList(), recipientNoticeLines(RecipientNotice(), { it }, translate))
        assertEquals(emptyList(), recipientNoticeLines(null, { it }, translate))
        val lines = recipientNoticeLines(
            RecipientNotice(listOf("bob", "carol"), listOf("carol" to UndeliverableReason.NOT_SET_UP)),
            { it.uppercase() }, translate)
        assertEquals(listOf(
            "e2ee.recipients.addednames=BOB, CAROL",
            "e2ee.undeliverable.title",
            "e2ee.undeliverable.notSetUpname=CAROL",
        ), lines)
    }
}
