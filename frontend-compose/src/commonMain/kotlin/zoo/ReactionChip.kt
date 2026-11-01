package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clipToBounds
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
            Hint(labels.joinToString(", ")) { hintModifier ->
                Row(
                    hintModifier
                        .heightIn(min = 32.dp)
                        .then(webControlModifier("button", enabled = editable, selected = mine))
                        .border(
                            1.dp,
                            if (mine) MaterialTheme.colorScheme.secondary.copy(alpha = .4f)
                            else MaterialTheme.colorScheme.outline,
                            RoundedCornerShape(16.dp),
                        )
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
                                    "emoji" to kitchenLabel(emoji),
                                    "count" to count.toString(),
                                )
                            selected = mine
                        }
                        .padding(horizontal = 8.dp, vertical = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    ReactionEmoji(emoji)
                    Text(
                        count.toString(),
                        Modifier.padding(start = 4.dp),
                        small = true,
                        color =
                            if (mine) MaterialTheme.colorScheme.onSecondaryContainer
                            else MaterialTheme.colorScheme.onBackground,
                    )
                }
            }
            // Keep the user list reachable by keyboard, touch and while reactions are disabled.
            Box(
                Modifier.size(
                        if (app.env.touch || LocalViewportWidth.current < 768f || focused) 40.dp
                        else 1.dp
                    )
                    .clipToBounds()
                    .onFocusChanged { focused = it.hasFocus }
            ) {
                IconButton(
                    "users-round",
                    app.t("reactions.showUsers", "emoji" to kitchenLabel(emoji)),
                    size = 40.dp,
                ) {
                    showUsers = true
                }
            }
        }
    }
    LaunchedEffect(rows) { if (rows.isEmpty()) showUsers = false }
    if (showUsers && rows.isNotEmpty())
        Panel(
            app.t("reactions.usersTitle", "emoji" to kitchenLabel(emoji)),
            { showUsers = false },
            width = 512.dp,
        ) {
            Column(
                Modifier.heightIn(max = 400.dp).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                labels.forEach { Text(it) }
            }
        }
}
