package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import zoo.android.ui.TimelineMediaKind
import zoo.android.ui.timelineMediaKind

class TimelineMediaPreviewTest {
    @Test fun recognizesImageAndVideoAttachmentsByMimeType() {
        assertEquals(TimelineMediaKind.Image, timelineMediaKind("image/jpeg"))
        assertEquals(TimelineMediaKind.Video, timelineMediaKind("VIDEO/MP4"))
        assertNull(timelineMediaKind("application/pdf"))
    }

    @Test fun allowsLocalVideoContainersAndRejectsPlaylists() {
        assertTrue(supportedVideoContainer(byteArrayOf(0, 0, 0, 24) + "ftypmp42".encodeToByteArray()))
        assertTrue(supportedVideoContainer("OggS00000000".encodeToByteArray()))
        assertTrue(supportedVideoContainer(byteArrayOf(0x1a, 0x45, 0xdf.toByte(), 0xa3.toByte(), 0, 0, 0, 0)))
        assertFalse(supportedVideoContainer("#EXTM3U\n#EXT-X-VERSION:3".encodeToByteArray()))
        assertFalse(supportedVideoContainer(byteArrayOf(0, 1)))
    }
}
