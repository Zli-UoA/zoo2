package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay

/** Wait for typing to pause before asking the server (each request may scan 12 months). */
private const val SEARCH_DEBOUNCE_MS = 350L

/** `/search?q=&type=`: server search of messages and files (docs/search.md). */
@Composable
fun GlobalSearchScreen(
    app: Session,
    users: List<Row>,
    memberships: List<Row>,
    controller: GlobalSearchController = app.globalSearch,
) {
    val initialQuery = remember { app.query["q"] ?: controller.query }
    val initialType = remember { app.query["type"]?.let(GlobalSearchType::from) ?: controller.type }
    var text by remember { mutableStateOf(initialQuery) }
    LaunchedEffect(controller) { controller.search(initialQuery, initialType) }
    LaunchedEffect(controller, text) {
        if (text.trim() == controller.query) return@LaunchedEffect
        delay(SEARCH_DEBOUNCE_MS)
        controller.search(text)
    }
    // Keep the route restorable (back from an opened result) without adding history steps.
    LaunchedEffect(controller.query, controller.type) {
        val path = globalSearchPath(controller.query, controller.type)
        if (isGlobalSearchPath(app.env.path) && app.env.path != path) app.platform.replace(path)
    }
    val touch = LocalTouchUi.current
    Column(
        Modifier.fillMaxSize().then(webControlModifier("region")).semantics {
            contentDescription = app.t("globalSearch.title")
        }
    ) {
        ScreenHeader("search", app.t("globalSearch.title"), primaryIcon = true)
        HorizontalDivider()
        Column(
            Modifier.fillMaxWidth().padding(
                horizontal = if (touch) AndroidDesign.screenInset else 16.dp,
                vertical = if (touch) AndroidDesign.spaceLarge else 12.dp,
            ),
            verticalArrangement = Arrangement.spacedBy(if (touch) AndroidDesign.spaceMedium else 8.dp),
        ) {
            Field(
                text,
                app.t("globalSearch.title"),
                Modifier.fillMaxWidth().testTag("global-search-field"),
                showLabel = false,
                placeholder = app.t("globalSearch.placeholder"),
                leadingIcon = "search",
                // Returning from an opened result should not reopen the keyboard.
                autoFocus = initialQuery.isEmpty(),
                inputRole = "searchbox",
            ) {
                text = it
            }
            Row(
                Modifier.fillMaxWidth()
                    .then(webControlModifier("tablist"))
                    .background(
                        MaterialTheme.colorScheme.surfaceVariant,
                        RoundedCornerShape(if (touch) AndroidDesign.surfaceRadius else 10.dp),
                    )
                    .padding(if (touch) AndroidDesign.spaceSmall else 4.dp),
                horizontalArrangement = Arrangement.spacedBy(if (touch) AndroidDesign.spaceSmall else 4.dp),
            ) {
                GlobalSearchType.entries.forEach { kind ->
                    Button(
                        app.t(kind.labelKey),
                        role = androidx.compose.ui.semantics.Role.Tab,
                        variant = "tab",
                        selected = controller.type == kind,
                        // Same segment surface as the Settings tabs: the touch chrome paints only a
                        // 32dp pill, so the selected 48dp segment carries the background itself.
                        modifier =
                            Modifier.weight(1f)
                                .background(
                                    if (controller.type == kind) MaterialTheme.colorScheme.background
                                    else androidx.compose.ui.graphics.Color.Transparent,
                                    RoundedCornerShape(if (touch) AndroidDesign.controlRadius else 8.dp),
                                )
                                .semantics { selected = controller.type == kind },
                    ) {
                        controller.search(text, kind)
                    }
                }
            }
            Text(app.t("globalSearch.encryptedNote"), small = true, muted = true)
        }
        HorizontalDivider()
        Box(Modifier.weight(1f).fillMaxWidth()) {
            GlobalSearchResults(app, controller, users, memberships)
        }
    }
}

@Composable
private fun GlobalSearchResults(
    app: Session,
    controller: GlobalSearchController,
    users: List<Row>,
    memberships: List<Row>,
) {
    val touch = LocalTouchUi.current
    when {
        controller.query.isEmpty() ->
            EmptyContentState("search", app.t("globalSearch.title"), app.t("globalSearch.prompt"))
        controller.loading -> TimelineLoadingState(app.t("globalSearch.loading"))
        controller.error && !controller.loaded ->
            LoadErrorState(app.t("globalSearch.loadError"), app.t("globalSearch.retry")) {
                controller.retry()
            }
        controller.items.isEmpty() && controller.nextCursor == null && !controller.error ->
            EmptyContentState("search", app.t("globalSearch.empty"), app.t("globalSearch.encryptedNote"))
        else -> {
            val channels by app.repo.channels.rows.collectAsState()
            val state = rememberLazyListState()
            LazyColumn(
                Modifier.fillMaxSize().then(webControlModifier("list")).semantics {
                    contentDescription = app.t("globalSearch.title")
                },
                state = state,
                contentPadding = PaddingValues(if (touch) AndroidDesign.screenInset else 16.dp),
                verticalArrangement = Arrangement.spacedBy(if (touch) AndroidDesign.spaceLarge else 12.dp),
            ) {
                items(controller.items, key = { "${controller.type.value}:${it.text("id")}" }) { row ->
                    GlobalSearchResult(app, row, controller.type, users, channels, memberships)
                }
                item("search-footer") {
                    GlobalSearchFooter(app, controller)
                }
            }
        }
    }
}

@Composable
private fun GlobalSearchFooter(app: Session, controller: GlobalSearchController) {
    val touch = LocalTouchUi.current
    val month = controller.searchedUntil.orEmpty()
    Column(
        Modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(if (touch) AndroidDesign.spaceMedium else 8.dp),
    ) {
        if (controller.items.isEmpty() && controller.nextCursor != null)
            Text(
                app.t("globalSearch.emptyOlder", "month" to month),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
        if (controller.error)
            LoadErrorState(app.t("globalSearch.loadError"), app.t("globalSearch.retry"), Modifier.testTag("global-search-error")) {
                controller.retry()
            }
        else if (controller.nextCursor != null) {
            // With no items, emptyOlder above already names the month.
            if (month.isNotEmpty() && controller.items.isNotEmpty())
                Text(app.t("globalSearch.searchedUntil", "month" to month), small = true, muted = true)
            if (controller.loadingOlder)
                Row(
                    Modifier.heightIn(min = if (touch) AndroidDesign.touchTarget else 32.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(if (touch) AndroidDesign.spaceLarge else 12.dp),
                ) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    Text(app.t("globalSearch.loading"), muted = true)
                }
            else
                Button(app.t("globalSearch.searchOlder"), variant = "outline") { controller.loadOlder() }
        }
    }
}

/** Matches NotificationCard: 12dp inset, 40dp avatar, 12dp gap, author and time, body, destination. */
@Composable
internal fun GlobalSearchResult(
    app: Session,
    row: Row,
    type: GlobalSearchType,
    users: List<Row>,
    channels: List<Row>,
    memberships: List<Row>,
) {
    val touch = LocalTouchUi.current
    val colors = MaterialTheme.colorScheme
    val destination = globalSearchDestination(row, type)
    val user =
        users.find { it.text("id") == row.text("user_id") }
            ?: row.nested("author").takeIf { it.isNotEmpty() }
    val name = user?.let(::author)?.ifEmpty { null } ?: row.text("user_id")
    val dm = row.flag("is_dm")
    val channel =
        channels.find { it.text("id") == row.text("channel_id") }
            ?: obj("id" to row.text("channel_id"), "channel_name" to row.text("channel_name"), "is_dm" to dm)
    val label = if (dm) dmTitle(app, channel, users, memberships) else "#${row.text("channel_name")}"
    val thread = row.present("parent_id")
    val file = type == GlobalSearchType.Files
    val body = if (file) row.text("file_name") else row.text("snippet")
    val summary =
        listOfNotNull(
                name,
                body,
                if (file) attachmentSize(row.text("file_size")) else null,
                if (thread) app.t("globalSearch.inThread") else null,
                label,
                app.platform.notificationTime(row.text("created_at")),
            )
            .joinToString(" · ")
    Row(
        Modifier.notificationSurface(read = true)
            .testTag("global-search-result")
            .nativeNavigation(destination)
            .semantics(mergeDescendants = true) { contentDescription = summary }
            .padding(
                horizontal = if (touch) AndroidDesign.feedInset else 12.dp,
                vertical = if (touch) AndroidDesign.postVerticalInset else 12.dp,
            ),
        horizontalArrangement = Arrangement.spacedBy(if (touch) AndroidDesign.avatarGap else 12.dp),
    ) {
        NotificationAvatar(
            size = if (touch) AndroidDesign.avatar else 40.dp,
            avatar = { Avatar(user, Modifier.testTag("global-search-avatar"), size = it) },
        ) {
            Icon(if (file) "paperclip" else "message-square-text", modifier = Modifier.size(12.dp))
        }
        Column(
            Modifier.weight(1f).testTag("global-search-body"),
            verticalArrangement = Arrangement.spacedBy(if (touch) AndroidDesign.spaceMedium else 4.dp),
        ) {
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Row(
                    Modifier.weight(1f),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        name,
                        Modifier.weight(1f, fill = false).alignByBaseline().testTag("global-search-author"),
                        weight = FontWeight.SemiBold,
                        maxLines = 1,
                    )
                    if (user?.flag("is_bot") == true)
                        Text(
                            "APP",
                            Modifier.background(colors.surfaceVariant, RoundedCornerShape(4.dp))
                                .padding(horizontal = 4.dp)
                                .alignByBaseline(),
                            size = 10.sp,
                            lineHeight = 15.sp,
                            muted = true,
                            weight = FontWeight.Medium,
                        )
                }
                NotificationTimestamp(app, row.text("created_at"))
            }
            if (file)
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Icon("paperclip", modifier = Modifier.size(16.dp))
                    Text(row.text("file_name"), Modifier.weight(1f, fill = false), weight = FontWeight.Medium, maxLines = 2)
                    Text(attachmentSize(row.text("file_size")), small = true, muted = true, maxLines = 1)
                }
            else
                Box(Modifier.fillMaxWidth()) {
                    CompositionLocalProvider(
                        LocalMfmStyle provides TextStyle(lineHeight = 22.75.sp),
                        LocalJoinedEmojis provides row.takeIf { it.containsKey("emojis") }?.let(::joinedEmojis),
                        // The whole card opens the message; keep taps on the snippet consistent.
                        LocalInlineLinksEnabled provides false,
                    ) {
                        RichText(body, users, maxLines = 3, selection = false)
                    }
                }
            NotificationDestination(
                if (dm) {
                    if (row.text("channel_name").startsWith("dm:group:")) "users" else "user-round"
                } else "hash",
                if (thread) app.t("globalSearch.inThread") else null,
                label,
                true,
            )
        }
    }
}
