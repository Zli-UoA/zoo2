package zoo.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DocumentPreviewTest {
    @Test fun pdfAttachmentsAreRecognizedByName() {
        assertTrue(isPdfAttachment("Report.PDF"))
        assertTrue(isPdfAttachment("notes.v2.pdf"))
        assertFalse(isPdfAttachment("pdf"))
        assertFalse(isPdfAttachment("slides.pptx"))
    }

    @Test fun onlyPdfContentIsOpened() {
        assertTrue(isPdfHeader("%PDF-1.7\n".toByteArray()))
        assertFalse(isPdfHeader("<html>".toByteArray()))
        assertFalse(isPdfHeader("%PD".toByteArray()))
    }

    @Test fun pagesStayWithinThePixelLimit() {
        // A4 縦を幅 1080px で描くときは、表示幅のまま。
        assertEquals(1080 to 1528, PdfPages.pageSize(595, 842, 1080))
        // 幅の上限。
        assertEquals(PdfPages.MAX_PAGE_WIDTH, PdfPages.pageSize(842, 595, 4000).first) // 横向きのページ
        // 幅の 4 倍の縦長ページは、幅を縮めて 1 枚 16 MiB までに収める。
        for ((w, h) in listOf(1000 to 4000, 100 to 100_000, 4000 to 100)) {
            val (width, height) = PdfPages.pageSize(w, h, 2400)
            assertTrue("$w x $h -> $width x $height", width.toLong() * height <= PdfPages.MAX_PAGE_PIXELS)
            assertTrue(width >= 1 && height >= 1)
        }
        // キャッシュ 6 枚が上限いっぱいでも、合計はバイト数の上限で抑える。
        assertTrue(PdfPages.MAX_CACHED_BYTES < PdfPages.MAX_CACHED_PAGES * PdfPages.MAX_PAGE_PIXELS * 4)
    }

    @Test fun accountSettingsOpenOnTheIssuerOrigin() {
        assertEquals("https://zoo.example/settings/profile", accountProfileUrl("https://zoo.example/api/auth"))
        assertEquals("http://localhost:8080/settings/profile", accountProfileUrl("http://localhost:8080/api/auth/"))
        assertNull(accountProfileUrl("zoo.example/api/auth"))
        assertNull(accountProfileUrl("javascript:alert(1)"))
    }
}
