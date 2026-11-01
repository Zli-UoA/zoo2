package zoo

import kotlin.test.*

class AttachmentsTest {
    @Test
    fun limitsAndMimeFallbackMatchTheReactContract() {
        fun file(size: Long, name: String = "image.PNG", type: String = "") =
            obj("name" to name, "type" to type, "size" to size.toString())
        assertNull(attachmentRejection(file(MAX_ATTACHMENT_BYTES), 9))
        assertEquals("tooLarge", attachmentRejection(file(MAX_ATTACHMENT_BYTES + 1), 0))
        assertEquals("tooMany", attachmentRejection(file(1), 10))
        assertEquals("unsupported", attachmentRejection(file(1, "bad.exe", "image/png"), 0))
        assertEquals("empty", attachmentRejection(file(0), 0))
        assertNull(attachmentRejection(file(1, "vector.svg", "image/svg+xml"), 0))
        assertEquals("image/png", attachmentType("image.PNG", "application/octet-stream"))
        assertEquals("image/jpeg", attachmentType("blob", " IMAGE/JPEG; charset=binary"))
        assertEquals("text/plain", attachmentType("file.txt", "text/plain"))
        assertEquals("application/pdf", attachmentType("file.pdf", "application/octet-stream"))
        assertEquals("application/octet-stream", attachmentType("blob", ""))
        assertNull(attachmentType("image.png", "application/x-sh"))
    }

    @Test
    fun signedUrlsRefreshBeforeExpirationAndMalformedExpiryUsesTheFallback() {
        val now = timestampMillis("2026-01-02T09:00:00Z")!!
        assertEquals(
            now + 60000,
            attachmentExpiry(
                "https://storage.test/file?X-Amz-Date=20260102T090000Z&X-Amz-Expires=60",
                now,
            ),
        )
        assertEquals(
            now + 240000,
            attachmentExpiry("https://storage.test/file?X-Amz-Expires=invalid", now),
        )
        assertEquals(now + 240000, attachmentExpiry("not a URL", now))
    }

    @Test
    fun filenamesKeepUnicodeCodepointsAndSizesMatchTheSharedPresentation() {
        assertEquals(
            "screen_shot.png",
            attachmentName(" screen\nshot.png ", "image/png", "fallback"),
        )
        assertEquals(
            "😀".repeat(251) + ".png",
            attachmentName("😀".repeat(256), "image/png", "fallback"),
        )
        assertEquals("attachment-fallback.jpg", attachmentName("", "image/jpeg", "fallback"))
        assertEquals("50 MB", attachmentSize(MAX_ATTACHMENT_BYTES.toString()))
        assertEquals("1.5 KB", attachmentSize("1536"))
        assertEquals("—", attachmentSize("invalid"))
    }
}
