package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Presentation of the Android shell's five root destinations. */
@Composable
internal fun MobileNavigation(
    path: String,
    unread: Int,
    label: (String) -> String,
    unreadLabel: String,
    navigate: (MobileDestination) -> Unit,
) {
    val selected = mobileDestination(path)
    val colors = MaterialTheme.colorScheme
    val dark = colors.background.red < .5f
    val bar = lerp(colors.background, colors.primary, if (dark) .06f else .04f)
    val indicator = lerp(colors.background, colors.primary, if (dark) .24f else .28f)
    Row(
        Modifier.fillMaxWidth().background(bar).selectableGroup()
            .semantics { contentDescription = label("nav.main") }
            .padding(horizontal = AndroidDesign.spaceSmall),
    ) {
        for (destination in MobileDestination.entries) {
            val active = selected == destination
            val title = label(destination.labelKey)
            val hasBadge = destination == MobileDestination.Notifications && unread > 0
            Column(
                Modifier.weight(1f).heightIn(min = 60.dp)
                    .selectable(active, role = Role.Tab) { navigate(destination) }
                    .semantics {
                        contentDescription = title
                        if (hasBadge) stateDescription = unreadLabel
                    }
                    .padding(vertical = AndroidDesign.spaceSmall),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Box(
                    Modifier.width(48.dp).height(28.dp)
                        .background(if (active) indicator else androidx.compose.ui.graphics.Color.Transparent, CircleShape),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(destination.icon, modifier = Modifier.size(20.dp),
                        color = if (active) colors.onBackground else colors.onSurfaceVariant)
                    if (hasBadge) {
                        Text(
                            if (unread > 99) "99+" else unread.toString(),
                            Modifier.align(Alignment.TopEnd).clearAndSetSemantics {}
                                .background(colors.primary, CircleShape).padding(horizontal = 4.dp),
                            size = 10.sp, lineHeight = 13.sp, color = colors.onPrimary,
                        )
                    }
                }
                Text(title, Modifier.clearAndSetSemantics {}, small = true, size = 11.sp, lineHeight = 15.sp,
                    maxLines = 2, textAlign = TextAlign.Center,
                    weight = if (active) FontWeight.SemiBold else FontWeight.Normal,
                    color = if (active) colors.onBackground else colors.onSurfaceVariant)
            }
        }
    }
}
