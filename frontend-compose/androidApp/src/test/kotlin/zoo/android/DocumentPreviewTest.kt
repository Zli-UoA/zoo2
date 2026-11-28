package zoo.android

import org.junit.Assert.assertFalse
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
}
