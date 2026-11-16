package zoo

import android.graphics.Color
import android.graphics.Paint
import android.graphics.pdf.PdfDocument
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import kotlin.test.*
import org.junit.runner.RunWith

/** PdfRenderer/PdfDocument need the Android PDF service, which Robolectric does not implement. */
@RunWith(AndroidJUnit4::class)
class NativePdfPreviewTest {
    private fun temporaryFile() = File.createTempFile("native-pdf-test-", ".pdf",
        ApplicationProvider.getApplicationContext<android.content.Context>().cacheDir)

    @Test fun rendersPdfPagesAndReusesTheDownloadedFile() {
        val file = temporaryFile()
        try {
            val pdf = PdfDocument()
            try {
                repeat(2) { index ->
                    val page = pdf.startPage(PdfDocument.PageInfo.Builder(300, 400, index + 1).create())
                    page.canvas.drawColor(if (index == 0) Color.RED else Color.BLUE)
                    page.canvas.drawRect(30f, 30f, 70f, 70f, Paint().apply { color = Color.BLACK })
                    pdf.finishPage(page)
                }
                file.outputStream().use { pdf.writeTo(it) }
            } finally { pdf.close() }
            val first = renderNativePdfPage(file, 0)
            val second = renderNativePdfPage(file, 1)
            try {
                assertEquals(2, first.pages)
                assertEquals(2, second.pages)
                assertTrue(first.bitmap.width <= 1600 && first.bitmap.height <= 2400)
                assertEquals(Color.RED, first.bitmap.getPixel(10, 10))
                assertEquals(Color.BLUE, second.bitmap.getPixel(10, 10))
                assertTrue(file.length() > 0, "Changing pages must reuse the source")
            } finally { first.bitmap.recycle(); second.bitmap.recycle() }
        } finally { file.delete() }
    }

}
