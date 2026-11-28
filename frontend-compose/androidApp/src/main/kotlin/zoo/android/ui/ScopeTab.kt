package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
internal fun ScopeTab(label: String, selected: Boolean, colors: AndroidColors, modifier: Modifier = Modifier, select: () -> Unit) {
    Box(modifier.selectable(selected, role = Role.Tab, onClick = select).heightIn(min = 40.dp),
        contentAlignment = Alignment.Center) {
        Text(label, Modifier.background(if (selected) colors.headerLight else Color.Transparent, CircleShape)
            .padding(horizontal = 12.dp, vertical = 3.dp), color = if (selected) colors.foreground else colors.secondary,
            fontSize = 14.sp, fontWeight = if (selected) AndroidWeights.emphasis else AndroidWeights.body)
    }
}
