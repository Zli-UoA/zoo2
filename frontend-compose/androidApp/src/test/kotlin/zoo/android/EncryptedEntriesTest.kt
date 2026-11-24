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
        val entry = encryptedEntry(base, encryptedRow, DecryptedBody(notice = "e2ee.undecryptable.keyMissing"), true, t)
        assertEquals("e2ee.undecryptable.keyMissing", entry.notice)
        assertEquals("", entry.body)
        assertTrue(entry.marks.isEmpty())
        assertFalse(entry.editable)
    }

    @Test
    fun senderAndRevisionWarningsAreMarksNextToTheBody() {
        val entry = encryptedEntry(base, encryptedRow, DecryptedBody("hello", "e2ee.sender.unverified"), true, t)
        assertEquals("hello", entry.body)
        assertNull(entry.notice)
        assertEquals(listOf("e2ee.sender.unverified"), entry.marks)
        // 暗号化した本文の URL は自動で取得しない。
        assertNull(entry.link)
        assertTrue(entry.editable)
        val stale = encryptedEntry(base, encryptedRow, DecryptedBody(revision = MessageRevision.STALE), true, t)
        assertEquals(listOf("e2ee.revision.stale"), stale.marks)
        assertFalse(stale.editable)
    }

    @Test
    fun plaintextHistoryIsShownButRowsWithAnEpochAndNoCiphertextAreNot() {
        val before = encryptedEntry(base.copy(id = "p", body = "before"), plainRow, null, true, t)
        assertEquals("before", before.body)
        assertFalse(before.editable)
        // epoch があるのに暗号文のない行は、本文を出さない。
        val forged = encryptedEntry(base.copy(body = "injected"), obj("id" to "m", "epoch" to "1", "content" to "injected"), null, true, t)
        assertEquals("", forged.body)
        assertEquals("e2ee.undecryptable.unencrypted", forged.notice)
        // 平文チャンネルでは何も変えない。
        assertEquals(base, encryptedEntry(base, plainRow, null, false, t))
    }

    @Test
    fun theBoundaryIsDecidedWithinTheDisplayedList() {
        val rows = mapOf("p1" to plainRow, "reply" to encryptedRow, "e1" to encryptedRow, "p2" to plainRow)
        // 会話の画面には返信（最初の暗号化したメッセージ）が出ない。表示する行の中で区切りを決める。
        val conversation = withEncryptionBoundary(
            listOf(base.copy(id = "p1"), base.copy(id = "e1"), base.copy(id = "p2")), rows, true, t)
        assertNull(conversation[0].divider)
        assertTrue(conversation[0].marks.isEmpty())
        assertEquals("e2ee.legacy.divider", conversation[1].divider)
        assertEquals(listOf("e2ee.legacy.unencryptedBadge"), conversation[2].marks)
        // 先頭が暗号化したメッセージなら区切りは出さない。
        assertNull(withEncryptionBoundary(listOf(base.copy(id = "e1")), rows, true, t)[0].divider)
        // 平文チャンネルでは何もしない。
        val plain = listOf(base.copy(id = "p1"), base.copy(id = "e1"))
        assertEquals(plain, withEncryptionBoundary(plain, rows, false, t))
    }

    @Test
    fun encryptedAttachmentsComeFromTheDecryptedPayload() {
        val info = obj("attachment_id" to "a1", "name" to "photo.png", "mimetype" to "image/png", "size" to 3, "file" to obj("v" to "v2"))
        val entry = encryptedEntry(base.copy(attachments = listOf("server" to "encrypted.bin")), encryptedRow,
            DecryptedBody("x", attachments = listOf(info)), true, t)
        assertEquals(listOf("a1" to "photo.png"), entry.attachments)
        assertEquals(info.toString(), entry.encryptedFiles["a1"])
    }

    @Test
    fun receivedAttachmentsFollowTheWebRules() {
        assertEquals(ReceivedAttachment("photo.png", "image/png", false), receivedAttachment(obj("name" to "photo.png", "mimetype" to "image/png")))
        // 平文の添付ならサーバーが拒否する種類は、取得も保存もしない。
        assertTrue(receivedAttachment(obj("name" to "x.sh", "mimetype" to "application/x-sh")).blocked)
        assertTrue(receivedAttachment(obj("name" to "payload.exe", "mimetype" to "application/x-msdownload")).blocked)
        // 末尾のドットや空白で拡張子を隠しても拒否する（サーバーの `is_disallowed_extension` と同じ）。
        assertTrue(receivedAttachment(obj("name" to "malware.exe. ", "mimetype" to "application/octet-stream")).blocked)
        assertTrue(receivedAttachment(obj("name" to "malware.EXE...", "mimetype" to "application/octet-stream")).blocked)
        // 双方向の制御文字で拡張子を偽装させない。
        assertEquals("evil_gnp.exe", receivedAttachment(obj("name" to "evil‮gnp.exe", "mimetype" to "image/png")).name)
    }

    @Test
    fun sharedFileNamesFitTheFileSystemLimit() {
        assertEquals("photo.png", sharedFileName("photo.png"))
        val long = sharedFileName("あ".repeat(80) + ".txt")
        assertTrue(long.toByteArray().size <= 255 - 37)
        assertTrue(long.endsWith(".txt"))
        assertTrue(long.removeSuffix(".txt").all { it == 'あ' })
        // 絵文字（サロゲートペア）の途中で切らない。
        val emoji = sharedFileName("😀".repeat(80) + ".pdf")
        assertTrue(emoji.toByteArray().size <= 255 - 37)
        assertEquals(emoji, String(emoji.toByteArray()))
        assertTrue(emoji.endsWith(".pdf"))
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
