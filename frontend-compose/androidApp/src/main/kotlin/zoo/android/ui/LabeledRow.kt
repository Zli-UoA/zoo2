package zoo.android.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
internal fun LabeledRow(
    title: String, detail: String?, colors: AndroidColors, modifier: Modifier = Modifier,
    titleWeight: FontWeight = AndroidWeights.body, leading: (@Composable () -> Unit)? = null,
    trailing: @Composable () -> Unit = {},
) {
    Row(Modifier.fillMaxWidth().then(modifier).heightIn(min = AndroidControlSize.target)
        .padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        leading?.invoke()
        Column(Modifier.weight(1f)) {
            Text(title, color = colors.foreground, fontSize = 16.sp, fontWeight = titleWeight)
            if (!detail.isNullOrEmpty()) Text(detail, color = colors.secondary, fontSize = 13.sp)
        }
        trailing()
    }
}
