package zoo

import kotlin.test.*

class KitchenBodyTest {
    @Test fun bodyShortcodesRoundTripWithoutChangingUnicodeSelections() {
        assertEquals(":kitchen_1f431_1f600:", kitchenShortcode("kitchen:😀_🐱"))
        assertEquals("kitchen:1f431_1f600", kitchenFromShortcode(":kitchen_1f600_1f431:"))
        assertEquals("😀", kitchenShortcode("😀"))
        assertNull(kitchenFromShortcode(":kitchen_d800_1f431:"))
        assertNull(kitchenFromShortcode(":kitchen_110000_1f431:"))
        assertEquals("emoji", parseInline(":kitchen_1f431_1f600:").single().kind)
        assertEquals("code", parseInline("`:kitchen_1f431_1f600:`").single().kind)
    }

    @Test fun emojiColumnsFitTheDialogAfterScreenInsets() {
        assertEquals(5, emojiPickerColumns(256f))
        assertEquals(8, emojiPickerColumns(416f))
        for (width in listOf(256f, 288f, 320f, 416f, 600f)) {
            assertTrue(emojiPickerColumns(width) * 48 <= width)
        }
    }
}
