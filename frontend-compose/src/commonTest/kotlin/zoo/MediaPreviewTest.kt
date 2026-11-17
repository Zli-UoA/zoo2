package zoo

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class MediaPreviewTest {
    @Test fun previewsMediaMailAndCommonDocuments() {
        for (mime in listOf("application/pdf", "video/mp4", "audio/ogg", "message/rfc822", "application/vnd.ms-outlook", "text/csv", "text/html", "application/zip", "application/vnd.openxmlformats-officedocument.wordprocessingml.document")) {
            assertTrue(hasMediaPreview(mime), mime)
        }
        assertTrue(hasMediaPreview("Application/PDF; charset=binary"))
        assertTrue(hasMediaPreview("application/octet-stream", "MAIL.EML"))
        assertTrue(hasMediaPreview("application/octet-stream", "report.docx"))
        assertFalse(hasMediaPreview("application/x-msdownload", "file.exe"))
        assertFalse(hasMediaPreview("application/octet-stream", "unknown.bin"))
        assertFalse(hasMediaPreview("image/svg+xml", "active.svg"))
    }
}
