package zoo.android.ui

import android.icu.text.BreakIterator
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.TextUnit

internal fun avatarInitial(name: String): String {
    val end = BreakIterator.getCharacterInstance().apply { setText(name) }.next()
    return if (end == BreakIterator.DONE) "" else name.substring(0, end)
}

@Composable
internal fun UserAvatar(name: String, image: ImageBitmap?, modifier: Modifier, background: Color, ink: Color,
    initialSize: TextUnit, weight: FontWeight = AndroidWeights.body) {
    Box(modifier.clip(CircleShape).background(background), contentAlignment = Alignment.Center) {
        if (image != null) Image(image, null, Modifier.matchParentSize(), contentScale = ContentScale.Crop)
        else {
            val initial = remember(name) { avatarInitial(name) }
            Text(initial, Modifier.wrapContentSize(unbounded = true).clearAndSetSemantics {},
                color = ink, fontSize = initialSize, lineHeight = initialSize * (16f / 13f), fontWeight = weight, maxLines = 1)
        }
    }
}
