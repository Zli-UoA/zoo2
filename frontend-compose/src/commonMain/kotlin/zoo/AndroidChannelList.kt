package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

internal data class AndroidChannelEntry(
    val id: String,
    val title: String,
    val subtitle: String,
    val icon: String,
    val unreadLabel: String? = null,
    val archivedLabel: String? = null,
    val avatar: Row? = null,
)

@Composable
internal fun AndroidChannelList(
    title: String,
    searchLabel: String,
    createLabel: String,
    joinedLabel: String,
    directLabel: String,
    createDirectLabel: String,
    emptyLabel: String,
    directEmptyLabel: String,
    joined: List<AndroidChannelEntry>,
    direct: List<AndroidChannelEntry>,
    loading: Boolean,
    error: (@Composable () -> Unit)?,
    open: (AndroidChannelEntry) -> Unit,
    browse: () -> Unit,
    create: () -> Unit,
    createDm: () -> Unit,
    modifier: Modifier,
) {
    val colors = MaterialTheme.colorScheme
    Column(modifier.fillMaxHeight().background(colors.background)) {
        ScreenHeader("hash", title, brand = true)
        LazyColumn(
            modifier = Modifier.weight(1f).fillMaxWidth(),
            contentPadding = PaddingValues(
                start = AndroidDesign.screenInset,
                end = AndroidDesign.screenInset,
                bottom = AndroidDesign.screenInset,
            ),
        ) {
            item {
                Row(
                    Modifier.fillMaxWidth()
                        .padding(top = AndroidDesign.spaceSmall, bottom = AndroidDesign.spaceMedium)
                        .background(colors.surfaceVariant, RoundedCornerShape(24.dp))
                        .clickable(role = Role.Button, onClick = browse)
                        .heightIn(min = AndroidDesign.touchTarget)
                        .padding(horizontal = AndroidDesign.spaceLarge),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(AndroidDesign.spaceMedium),
                ) {
                    Icon("search", modifier = Modifier.size(20.dp))
                    Text(searchLabel)
                }
            }
            item { ChannelListSection(joinedLabel, createLabel, create) }
            if (error != null) item { error() }
            when {
                loading && joined.isEmpty() -> items(4) {
                    Box(Modifier.fillMaxWidth().padding(vertical = AndroidDesign.spaceSmall)
                        .height(AndroidDesign.touchTarget).background(colors.surfaceVariant, RoundedCornerShape(8.dp)))
                }
                joined.isEmpty() && error == null -> item { Text(emptyLabel, muted = true) }
                else -> items(joined, key = { it.id }) { AndroidChannelListRow(it, open) }
            }
            item { ChannelListSection(directLabel, createDirectLabel, createDm) }
            if (direct.isEmpty() && !loading) item { Text(directEmptyLabel, muted = true) }
            items(direct, key = { it.id }) { AndroidChannelListRow(it, open) }
        }
    }
}

@Composable
private fun ChannelListSection(title: String, action: String, onAction: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(top = AndroidDesign.spaceSmall),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(title, Modifier.weight(1f), small = true, muted = true)
        IconButton("plus", action, click = onAction)
    }
}

@Composable
private fun AndroidChannelListRow(entry: AndroidChannelEntry, open: (AndroidChannelEntry) -> Unit) {
    val colors = MaterialTheme.colorScheme
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp)
            .clickable(role = Role.Button) { open(entry) }
            .semantics { contentDescription = entry.title }
            .padding(horizontal = AndroidDesign.spaceSmall, vertical = AndroidDesign.spaceMedium),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(AndroidDesign.spaceLarge),
    ) {
        Box(Modifier.size(32.dp), contentAlignment = Alignment.Center) {
            if (entry.avatar != null) Avatar(entry.avatar, size = 32.dp)
            else Icon(entry.icon, modifier = Modifier.size(20.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(entry.title, weight = androidx.compose.ui.text.font.FontWeight.SemiBold, maxLines = 1)
            if (entry.subtitle.isNotBlank()) Text(entry.subtitle, small = true, muted = true, maxLines = 1)
        }
        (entry.unreadLabel ?: entry.archivedLabel)?.let {
            Text(it, Modifier.background(colors.secondaryContainer, CircleShape)
                .padding(horizontal = AndroidDesign.spaceMedium), small = true, color = colors.onSecondaryContainer)
        }
    }
    HorizontalDivider(color = colors.outlineVariant)
}
