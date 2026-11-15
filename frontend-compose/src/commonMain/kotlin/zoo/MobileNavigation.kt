package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Shares the shell's routes and unread state; no independent data subscriptions. */
@Composable
internal fun MobileNavigation(
    path: String,
    unread: Int,
    label: (String) -> String,
    unreadLabel: String,
    navigate: (MobileDestination) -> Unit,
) {
    val selected = mobileDestination(path)
    NavigationBar(
        containerColor = MaterialTheme.colorScheme.surface,
        tonalElevation = 0.dp,
        // MainActivity already owns system bar and IME insets.
        windowInsets = WindowInsets(0, 0, 0, 0),
        modifier = Modifier.semantics { contentDescription = label("nav.main") },
    ) {
        MobileDestination.entries.forEach { destination ->
            val title = label(destination.labelKey)
            NavigationBarItem(
                selected = selected == destination,
                onClick = { navigate(destination) },
                modifier = Modifier.semantics {
                    contentDescription = title
                    if (destination == MobileDestination.Notifications && unread > 0)
                        stateDescription = unreadLabel
                },
                icon = {
                    BadgedBox(badge = {
                        if (destination == MobileDestination.Notifications && unread > 0)
                            Badge { androidx.compose.material3.Text(if (unread > 99) "99+" else unread.toString()) }
                    }) {
                        Icon(destination.icon, modifier = Modifier.size(24.dp))
                    }
                },
                label = { Text(title, small = true, size = 12.sp, maxLines = 1) },
                colors = NavigationBarItemDefaults.colors(
                    indicatorColor = MaterialTheme.colorScheme.secondaryContainer,
                    selectedIconColor = MaterialTheme.colorScheme.onSecondaryContainer,
                    selectedTextColor = MaterialTheme.colorScheme.onSurface,
                ),
            )
        }
    }
}
