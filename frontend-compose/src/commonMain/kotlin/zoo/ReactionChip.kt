package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.*

fun reactionUserLabel(user: Row): String =
    if (user.text("display_name").isNotEmpty())
        "${user.text("display_name")} (@${user.text("user_name")})"
    else "@${user.text("user_name")}"

fun reactionUserLabels(rows: List<Row>, users: List<Row>, unavailable: String): List<String> {
    val byId = users.associateBy { it.text("id") }
    val ids = rows.map { it.text("user_id") }.distinct()
    return ids.mapNotNull { byId[it]?.let(::reactionUserLabel) } +
        if (ids.any { it !in byId }) listOf(unavailable) else emptyList()
}

@Composable
fun ReactionChip(
    app: Session,
    emoji: String,
    rows: List<Row>,
    users: List<Row>,
    editable: Boolean,
    wide: Boolean,
    toggle: () -> Unit,
) {
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    var focused by remember { mutableStateOf(false) }
    var showUsers by remember(emoji) { mutableStateOf(false) }
    val loading by app.repo.users.loading.collectAsState()
    val error by app.repo.users.error.collectAsState()
    val labels =
        reactionUserLabels(
            rows,
            users,
            app.t(
                if (loading && error == null) "reactions.usersLoading"
                else "reactions.userUnavailable"
            ),
        )
    val count = rows.map { it.text("user_id") }.distinct().size
    val mine = rows.any { it.text("user_id") == app.config.userId }
    Box {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Row(
                Modifier.heightIn(min = 32.dp)
                    .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(16.dp))
                    .background(
                        if (mine) MaterialTheme.colorScheme.secondaryContainer
                        else MaterialTheme.colorScheme.surface,
                        RoundedCornerShape(16.dp),
                    )
                    .hoverable(interaction)
                    .onFocusChanged { focused = it.hasFocus }
                    .clickable(role = Role.Button) { if (editable) toggle() }
                    .semantics {
                        contentDescription =
                            app.t(
                                if (mine) "reactions.chipMine" else "reactions.chip",
                                "emoji" to emoji,
                                "count" to count.toString(),
                            )
                        selected = mine
                    }
                    .padding(horizontal = 8.dp, vertical = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ReactionEmoji(emoji)
                Text(count.toString(), Modifier.padding(start = 4.dp), small = true)
            }
            // Keep the user list reachable by keyboard, touch and while reactions are disabled.
            IconButton("users", app.t("reactions.showUsers", "emoji" to emoji), size = 40.dp) {
                showUsers = true
            }
        }
        if (wide && (hovered || focused))
            Box(
                Modifier.offset(y = 40.dp)
                    .background(MaterialTheme.colorScheme.surfaceVariant)
                    .padding(8.dp)
            ) {
                Text(labels.joinToString(", "), small = true)
            }
    }
    LaunchedEffect(rows) { if (rows.isEmpty()) showUsers = false }
    if (showUsers && rows.isNotEmpty())
        Panel(app.t("reactions.usersTitle", "emoji" to emoji), { showUsers = false }) {
            Column(
                Modifier.heightIn(max = 400.dp).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                labels.forEach { Text(it) }
            }
        }
}
