package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.key.*
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.serialization.json.*

fun validChannelName(name: String): Boolean =
    name !in setOf("all", "channel", "everyone", "here", "general") &&
        name.isNotBlank() &&
        name.codePointCount() <= 80 &&
        channelCharactersValid(name)

@Composable
fun ChannelDialog(
    app: Session,
    kind: String,
    channel: Row?,
    channels: List<Row>,
    users: List<Row>,
    members: List<Row>,
    initial: Row = JsonObject(emptyMap()),
    open: Boolean = true,
    onOpenChange: ((Boolean) -> Unit)? = null,
) {
    val initialName =
        remember(kind) {
            initial.text("name").ifEmpty {
                if (kind == "rename") channel?.text("channel_name").orEmpty() else ""
            }
        }
    val initialDescription =
        remember(kind) {
            initial.text("description").ifEmpty {
                if (kind == "rename") channel?.text("description").orEmpty() else ""
            }
        }
    var name by remember(kind) { mutableStateOf(initialName) }
    var description by remember(kind) { mutableStateOf(initialDescription) }
    var private by
        remember(kind) {
            mutableStateOf(
                initial.flag("private") || kind == "rename" && channel?.flag("is_private") == true
            )
        }
    var search by remember(kind) { mutableStateOf(initial.text("search")) }
    var selected by
        remember(kind) {
            mutableStateOf<Set<String>>(
                initial.array("selected").map { it.jsonPrimitive.content }.toSet()
            )
        }
    var created by remember(kind) { mutableStateOf<String?>(null) }
    var limitReached by remember(kind) { mutableStateOf(false) }
    var activeCandidate by remember(kind) { mutableStateOf(0) }
    val busy = "channel-dialog" in app.pending
    fun dismiss() {
        when (kind) {
            "create" -> {
                name = ""
                description = ""
                private = false
            }
            "dm" -> {
                search = ""
                selected = emptySet()
                created = null
                limitReached = false
                activeCandidate = 0
            }
            "browse" -> search = ""
        }
        app.dialog = null
        onOpenChange?.invoke(false)
    }
    fun close() {
        if (!busy) dismiss()
    }
    val title =
        when (kind) {
            "create" -> "channels.createTitle"
            "rename" -> "channels.editTitle"
            "dm" -> "dms.createTitle"
            "browse" -> "channels.browseTitle"
            "leave" -> "dms.leaveTitle"
            else ->
                if (channel?.present("archive_at") == true) "moderation.unarchiveTitle"
                else "moderation.archiveTitle"
        }
    fun saveChannel() {
        app.run("channel-dialog") {
            val body = buildJsonObject {
                if (kind == "create" || name.trim() != initialName.trim())
                    put("channel_name", name.trim())
                if (kind == "create") {
                    put(
                        "description",
                        description.trim().takeIf { it.isNotEmpty() }?.let(::JsonPrimitive)
                            ?: JsonNull,
                    )
                    put("is_private", private)
                } else if (description.trim() != initialDescription.trim()) {
                    put("description", description.trim())
                }
            }
            app.repo.channels.write(
                app.api,
                if (kind == "create") "/api/channels" else "/api/channels/${channel!!.text("id")}",
                if (kind == "create") "POST" else "PATCH",
                body,
            )
            dismiss()
        }
    }
    fun submitDm() {
        app.run("channel-dialog") {
            val id =
                created
                    ?: run {
                        val response =
                            app.api
                                .request(
                                    "/api/dms",
                                    "POST",
                                    buildJsonObject {
                                        put(
                                            "target_user_ids",
                                            JsonArray(selected.map(::JsonPrimitive)),
                                        )
                                    },
                                )
                                .jsonObject
                        response.getValue("data").jsonObject.text("id")
                    }
            try {
                withTimeout(8_000) {
                    combine(app.repo.channels.rows, app.repo.members.rows) { channels, members ->
                            channels.any { it.text("id") == id } &&
                                isMember(members, id, app.config.userId)
                        }
                        .first { it }
                }
                dismiss()
                app.navigate("/channels/$id")
            } catch (_: TimeoutCancellationException) {
                created = id
            }
        }
    }
    val channelAction =
        if (kind == "leave") "leave"
        else if (channel?.present("archive_at") == true) "unarchive" else "archive"
    fun confirmChannelAction() {
        val target = channel ?: return
        app.run("channel-dialog") {
            (if (channelAction == "leave") app.repo.members else app.repo.channels).write(
                app.api,
                "/api/channels/${target.text("id")}/$channelAction",
                "POST",
            )
            dismiss()
            if (channelAction == "leave") app.navigate("/")
        }
    }
    if (!open) return
    val formFooter: (@Composable () -> Unit)? =
        if (kind == "create" || kind == "rename") {
            {
                DialogFooter(
                    listOf(
                        DialogAction(app.t("common.cancel"), enabled = !busy, action = ::close),
                        DialogAction(
                            app.t(if (kind == "create") "channels.create" else "common.save"),
                            "default",
                            !busy &&
                                app.env.online &&
                                name.trim().isNotEmpty() &&
                                description.trim().codePointCount() <= 250 &&
                                (kind == "create" ||
                                    name.trim() != initialName.trim() ||
                                    description.trim() != initialDescription.trim()),
                            ::saveChannel,
                        ),
                    )
                )
            }
        } else if (kind == "dm") {
            {
                DialogFooter(
                    listOf(
                        DialogAction(app.t("common.cancel"), enabled = !busy, action = ::close),
                        DialogAction(
                            app.t(
                                if (created != null) if (busy) "dms.opening" else "dms.openCreated"
                                else if (busy) "dms.creating" else "dms.create"
                            ),
                            "default",
                            app.env.online && !busy && selected.isNotEmpty(),
                            ::submitDm,
                        ),
                    ),
                    minimumActionHeight = 44.dp,
                )
            }
        } else if (kind == "leave" || kind == "moderate") {
            {
                DialogFooter(
                    listOf(
                        DialogAction(
                            app.t("common.cancelShort"),
                            enabled = !busy,
                            action = ::close,
                        ),
                        DialogAction(
                            app.t(
                                if (busy)
                                    if (kind == "leave") "dms.leaving" else "moderation.executing"
                                else if (kind == "leave") "dms.leave" else "moderation.execute"
                            ),
                            if (channelAction == "unarchive") "default" else "destructive",
                            !busy && app.env.online,
                            ::confirmChannelAction,
                        ),
                    ),
                    minimumActionHeight = if (kind == "leave") 44.dp else 0.dp,
                )
            }
        } else null
    Panel(
        app.t(title),
        ::close,
        width = if (kind in setOf("browse", "dm")) 448.dp else 384.dp,
        footer = formFooter,
        onEscape = {
            if (!busy && kind in setOf("browse", "dm") && search.isNotEmpty()) {
                search = ""
                activeCandidate = 0
                true
            } else false
        },
        alert = kind == "leave" || kind == "moderate",
        description =
            if (kind == "create") app.t("channels.visibilityHelp")
            else if (kind == "rename") app.t("channels.editHelp")
            else if (kind == "browse") app.t("channels.browseDescription")
            else if (kind == "dm") app.t("dms.createDescription")
            else if (kind == "leave") app.t("dms.leaveDescription")
            else if (kind == "moderate") app.t("moderation.${channelAction}Description") else "",
    ) {
        when (kind) {
            "create",
            "rename" -> {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Field(
                        name,
                        app.t("channels.newName"),
                        showLabel = false,
                        placeholder = app.t("channels.newName"),
                        autoFocus = true,
                    ) {
                        name = it
                    }
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                        ) {
                            Text(
                                app.t("channels.descriptionLabel"),
                                Modifier.alignByBaseline(),
                                weight = FontWeight.Medium,
                            )
                            Text(
                                app.t(
                                    "channels.descriptionCounter",
                                    "length" to description.trim().codePointCount().toString(),
                                    "max" to "250",
                                ),
                                modifier = Modifier.alignByBaseline(),
                                muted = description.trim().codePointCount() <= 250,
                                small = true,
                            )
                        }
                        Field(
                            description,
                            app.t("channels.descriptionLabel"),
                            singleLine = false,
                            showLabel = false,
                            placeholder = app.t("channels.descriptionPlaceholder"),
                        ) {
                            description = it
                        }
                        Text(
                            app.t(
                                if (description.trim().codePointCount() > 250)
                                    "channels.descriptionTooLong"
                                else "channels.descriptionHelp",
                                "length" to description.trim().codePointCount().toString(),
                                "max" to "250",
                            ),
                            muted = true,
                            small = true,
                        )
                    }
                    if (kind == "create")
                        Check(app.t("channels.makePrivate"), private, !busy) { private = it }
                }
            }
            "browse" -> {
                ChannelSearchField(
                    search,
                    app.t("channels.browseSearch"),
                    app.t("channels.searchClear"),
                    compactClear = true,
                ) {
                    search = it
                }
                Box(Modifier.fillMaxWidth().height(320.dp)) {
                    val filtered =
                        channels
                            .filterNot { it.present("deleted_at") }
                            .filter {
                                !it.flag("is_dm") &&
                                    !it.flag("is_private") &&
                                    !(if ("joinedIds" in initial)
                                        it.text("id") in
                                            initial.array("joinedIds").map { v ->
                                                v.jsonPrimitive.content
                                            }
                                    else isMember(members, it.text("id"), app.config.userId)) &&
                                    it.text("channel_name").contains(search.trim(), true)
                            }
                    if (filtered.isEmpty())
                        Text(
                            app.t(
                                if (search.isBlank()) "channels.browseEmpty"
                                else "channels.searchEmpty"
                            ),
                            Modifier.align(Alignment.Center).padding(8.dp),
                            muted = true,
                        )
                    else
                        LazyColumn(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            items(filtered, key = { it.text("id") }) { row ->
                                Row(
                                    Modifier.fillMaxWidth()
                                        .heightIn(min = 56.dp)
                                        .then(webControlModifier("link"))
                                        .clickable {
                                            dismiss()
                                            app.navigate("/channels/${row.text("id")}")
                                        }
                                        .padding(horizontal = 12.dp, vertical = 8.dp),
                                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Icon("hash", modifier = Modifier.size(16.dp))
                                    Column(Modifier.weight(1f)) {
                                        Text(row.text("channel_name"), maxLines = 1)
                                        if (row.text("description").isNotBlank())
                                            Text(
                                                row.text("description").trim(),
                                                muted = true,
                                                small = true,
                                                maxLines = 1,
                                            )
                                    }
                                    if (row.present("archive_at")) ChannelArchiveBadge(app)
                                    Text(
                                        app.t("channels.view"),
                                        small = true,
                                        color = MaterialTheme.colorScheme.secondary,
                                        weight = FontWeight.Medium,
                                    )
                                }
                            }
                        }
                }
            }

            "dm" -> {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    val candidates =
                        users
                            .filter {
                                it.text("id") != app.config.userId &&
                                    !it.flag("is_bot") &&
                                    it.text("id") !in selected &&
                                    (author(it).contains(search.trim(), true) ||
                                        it.text("user_name").contains(search.trim(), true))
                            }
                            .sortedWith { left, right ->
                                compareLocaleText(author(left), author(right), app.env.locale)
                            }
                    fun selectUser(user: Row) {
                        if (selected.size >= 9) {
                            limitReached = true
                        } else {
                            limitReached = false
                            selected = selected + user.text("id")
                            search = ""
                            activeCandidate = 0
                        }
                    }
                    ChannelSearchField(
                        search,
                        app.t("dms.search"),
                        app.t("dms.searchClear"),
                        inputRole = "combobox",
                        enabled = !busy && created == null,
                        modifier =
                            Modifier.onPreviewKeyEvent {
                                if (it.type != KeyEventType.KeyDown) false
                                else
                                    when (it.key) {
                                        Key.DirectionDown -> {
                                            activeCandidate =
                                                (activeCandidate + 1).coerceAtMost(
                                                    (candidates.size - 1).coerceAtLeast(0)
                                                )
                                            true
                                        }
                                        Key.DirectionUp -> {
                                            activeCandidate = (activeCandidate - 1).coerceAtLeast(0)
                                            true
                                        }
                                        Key.MoveHome -> {
                                            activeCandidate = 0
                                            true
                                        }
                                        Key.MoveEnd -> {
                                            activeCandidate = candidates.lastIndex.coerceAtLeast(0)
                                            true
                                        }
                                        Key.Enter -> {
                                            candidates
                                                .getOrNull(activeCandidate)
                                                ?.let(::selectUser) != null
                                        }
                                        else -> false
                                    }
                            },
                    ) {
                        search = it
                        activeCandidate = 0
                    }
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                        ) {
                            Text(
                                app.t("dms.selected"),
                                muted = true,
                                small = true,
                                weight = FontWeight.Medium,
                            )
                            Text(
                                app.t(
                                    "dms.selectedCount",
                                    "count" to selected.size.toString(),
                                    "max" to "9",
                                ),
                                small = true,
                            )
                        }
                        if (selected.isEmpty())
                            Text(app.t("dms.selectHelp"), small = true, muted = true)
                        else
                            FlowRow(
                                horizontalArrangement = Arrangement.spacedBy(6.dp),
                                verticalArrangement = Arrangement.spacedBy(6.dp),
                            ) {
                                selected.forEach { id ->
                                    val user = users.find { it.text("id") == id }
                                    val label = user?.let(::author) ?: id
                                    Row(
                                        Modifier.heightIn(min = 44.dp)
                                            .border(
                                                1.dp,
                                                MaterialTheme.colorScheme.outline,
                                                RoundedCornerShape(50),
                                            )
                                            .then(webControlModifier("button"))
                                            .clickable(enabled = !busy && created == null) {
                                                selected = selected - id
                                                limitReached = false
                                            }
                                            .semantics {
                                                contentDescription =
                                                    app.t("dms.remove", "name" to label)
                                            }
                                            .padding(
                                                start = 4.dp,
                                                end = 8.dp,
                                                top = 4.dp,
                                                bottom = 4.dp,
                                            ),
                                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        Avatar(user, size = 24.dp)
                                        Text(label)
                                        Icon("x", modifier = Modifier.size(14.dp))
                                    }
                                }
                            }
                        Text(app.t("dms.memberLimit", "max" to "10"), small = true, muted = true)
                    }
                    if (limitReached)
                        Text(
                            app.t("dms.limitReached"),
                            Modifier.then(webControlModifier("status")),
                            small = true,
                            color = MaterialTheme.colorScheme.error,
                        )
                    BoxWithConstraints(Modifier.fillMaxWidth().height(256.dp)) {
                        val usernameMaxWidth = (maxWidth - 8.dp) * .4f
                        if (candidates.isEmpty())
                            Text(
                                app.t(
                                    if (users.isEmpty()) "dms.usersEmpty"
                                    else if (search.isBlank()) "dms.candidatesEmpty"
                                    else "dms.noResults"
                                ),
                                Modifier.align(Alignment.Center).padding(horizontal = 8.dp),
                                muted = true,
                                textAlign = TextAlign.Center,
                            )
                        else
                            LazyColumn(
                                Modifier.fillMaxSize()
                                    .padding(end = 8.dp)
                                    .then(webControlModifier("listbox"))
                            ) {
                                itemsIndexed(candidates, key = { _, user -> user.text("id") }) {
                                    index,
                                    user ->
                                    Row(
                                        Modifier.fillMaxWidth()
                                            .heightIn(min = 44.dp)
                                            .background(
                                                if (index == activeCandidate)
                                                    if (app.env.dark)
                                                        androidx.compose.ui.graphics.Color(
                                                            0xff2e2e2e
                                                        )
                                                    else
                                                        androidx.compose.ui.graphics.Color(
                                                            0xfffffbe8
                                                        )
                                                else androidx.compose.ui.graphics.Color.Transparent,
                                                RoundedCornerShape(10.dp),
                                            )
                                            .then(
                                                webControlModifier(
                                                    "option",
                                                    selected = index == activeCandidate,
                                                )
                                            )
                                            .clickable(enabled = !busy && created == null) {
                                                selectUser(user)
                                            }
                                            .padding(horizontal = 8.dp, vertical = 6.dp),
                                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        Avatar(user, size = 24.dp)
                                        Text(author(user), Modifier.weight(1f), maxLines = 1)
                                        if (author(user) != user.text("user_name"))
                                            Text(
                                                "@${user.text("user_name")}",
                                                modifier = Modifier.widthIn(max = usernameMaxWidth),
                                                muted = true,
                                                small = true,
                                                maxLines = 1,
                                            )
                                        Icon("plus", modifier = Modifier.size(16.dp))
                                    }
                                }
                            }
                    }
                    if (created != null)
                        Text(
                            app.t("dms.syncPending"),
                            Modifier.then(webControlModifier("status")),
                            small = true,
                            color = MaterialTheme.colorScheme.error,
                        )
                }
            }
            "leave",
            "moderate" -> Unit
        }
    }
}

/** Shared browser-style search control used by channel discovery and DM creation. */
@Composable
private fun ChannelSearchField(
    value: String,
    label: String,
    clearLabel: String,
    inputRole: String = "searchbox",
    enabled: Boolean = true,
    compactClear: Boolean = false,
    modifier: Modifier = Modifier,
    change: (String) -> Unit,
) {
    Box(Modifier.fillMaxWidth().height(32.dp)) {
        Field(
            value,
            label,
            modifier =
                modifier.fillMaxWidth().onPreviewKeyEvent {
                    if (
                        enabled &&
                            it.type == KeyEventType.KeyDown &&
                            it.key == Key.Escape &&
                            value.isNotEmpty()
                    ) {
                        change("")
                        true
                    } else false
                },
            showLabel = false,
            placeholder = label,
            inputRole = inputRole,
            leadingIcon = "search",
            autoFocus = true,
            contentPadding = PaddingValues(start = 32.dp, end = 32.dp, top = 6.dp, bottom = 6.dp),
            enabled = enabled,
            change = change,
        )
        if (value.isNotEmpty())
            Button(
                clearLabel,
                enabled,
                variant = "ghost",
                size = if (compactClear) "icon-xs" else "icon",
                icon = "x",
                title = clearLabel,
                modifier =
                    Modifier.align(Alignment.CenterEnd)
                        .offset(x = if (compactClear) (-4).dp else 0.dp)
                        .requiredSize(if (compactClear) 24.dp else 36.dp),
            ) {
                change("")
            }
    }
}

@Composable
fun Members(
    app: Session,
    channel: Row,
    users: List<Row>,
    memberships: List<Row>,
    modifier: Modifier,
    initialCandidatesOpen: Boolean = false,
    asScreen: Boolean = false,
    onBack: (() -> Unit)? = null,
) {
    val id = channel.text("id")
    val ids = memberships.filter { it.text("channel_id") == id }.map { it.text("user_id") }.toSet()
    val joined = app.config.userId in ids
    val creator = channel.text("created_user_id") == app.config.userId
    val canView = !channel.flag("is_private") || joined
    val manage = canManage(channel, app.config.userId, app.config.isAdmin)
    val writable = app.env.online && !channel.present("archive_at")
    val canAdd =
        writable && !channel.flag("is_dm") && (manage || !channel.flag("is_private") && joined)
    var search by remember(id) { mutableStateOf("") }
    var candidatesOpen by remember(id) { mutableStateOf(initialCandidatesOpen) }
    var candidateAnchor by remember { mutableStateOf<Rect?>(null) }
    var candidateIndex by remember(id) { mutableStateOf(0) }
    val candidates =
        users
            .filter {
                !it.flag("is_bot") &&
                    it.text("id") !in ids &&
                    (author(it).contains(search, true) ||
                        it.text("user_name").contains(search, true))
            }
            .sortedWith { left, right ->
                compareLocaleText(author(left), author(right), app.env.locale)
            }
            .take(8)
    fun addCandidate(person: Row) {
        app.run("member-${person.text("id")}") {
            app.repo.members.write(
                app.api,
                "/api/channels/$id/users",
                "POST",
                obj("user_id" to person.text("id")),
            )
            search = ""
            candidatesOpen = false
        }
    }
    var kick by remember(id) { mutableStateOf<Row?>(null) }
    Column(modifier.fillMaxHeight()) {
        PaneHeader {
            if (asScreen)
                IconButton("arrow-left", app.t("members.back"), size = 36.dp) {
                    if (onBack != null) onBack() else app.membersOpen = false
                }
            Column(Modifier.weight(1f)) {
                Text(
                    app.t("members.title"),
                    Modifier.then(
                            webControlModifier(
                                "heading",
                                attributes = mapOf("aria-level" to if (asScreen) "1" else "3"),
                            )
                        )
                        .semantics { heading() },
                    weight = FontWeight.SemiBold,
                )
                if (asScreen) Text("#${channel.text("channel_name")}", muted = true, small = true)
            }
            Text(
                if (canView) users.count { it.text("id") in ids && !it.flag("is_bot") }.toString()
                else "—",
                Modifier.background(
                        MaterialTheme.colorScheme.surfaceVariant,
                        androidx.compose.foundation.shape.RoundedCornerShape(4.dp),
                    )
                    .padding(horizontal = 8.dp, vertical = 2.dp),
                small = true,
                muted = true,
            )
        }
        HorizontalDivider()
        if (!canView)
            Box(
                Modifier.weight(1f).fillMaxWidth().padding(24.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(app.t("members.private"), muted = true, small = true)
            }
        else {
            if (channel.present("archive_at"))
                Text(
                    app.t("moderation.archivedMembersHelp"),
                    Modifier.fillMaxWidth()
                        .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .4f))
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                    muted = true,
                    small = true,
                )
            LazyColumn(
                Modifier.weight(1f).fillMaxWidth().padding(8.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                items(
                    users
                        .filter { it.text("id") in ids }
                        .sortedWith { left, right ->
                            val creatorId = channel.text("created_user_id")
                            when {
                                left.text("id") == right.text("id") -> 0
                                left.text("id") == creatorId -> -1
                                right.text("id") == creatorId -> 1
                                else ->
                                    compareLocaleText(
                                            author(left),
                                            author(right),
                                            "en",
                                            numeric = true,
                                            baseSensitivity = true,
                                        )
                                        .takeIf { it != 0 }
                                        ?: compareLocaleText(
                                            left.text("user_name"),
                                            right.text("user_name"),
                                            "en",
                                            numeric = true,
                                            baseSensitivity = true,
                                        )
                            }
                        },
                    key = { it.text("id") },
                ) { person ->
                    Row(
                        Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp),
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Avatar(person, size = 24.dp)
                        Column(Modifier.weight(1f)) {
                            Text(author(person))
                            Text("@${person.text("user_name")}", small = true, muted = true)
                        }
                        if (person.text("id") == channel.text("created_user_id"))
                            Text(app.t("members.creator"), small = true, muted = true)
                        else if (person.text("id") == app.config.userId)
                            Text(app.t("members.you"), small = true, muted = true)
                        else if (
                            manage && person.text("id") !in app.admins && !channel.flag("is_dm")
                        )
                            IconButton(
                                "log-out",
                                app.t("moderation.kickLabel", "name" to author(person)),
                                writable,
                                size = 28.dp,
                            ) {
                                kick = person
                            }
                    }
                }
            }
            HorizontalDivider()
            Column(
                Modifier.fillMaxWidth().padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (canAdd) {
                    Field(
                        search,
                        app.t("members.addLabel"),
                        showLabel = false,
                        placeholder = app.t("members.search"),
                        inputRole = "combobox",
                        onBlur = { candidatesOpen = false },
                        leadingIcon = "search",
                        modifier =
                            Modifier.onPreviewKeyEvent {
                                    if (it.type != KeyEventType.KeyDown) false
                                    else
                                        when (it.key) {
                                            Key.DirectionDown -> {
                                                if (candidatesOpen)
                                                    candidateIndex =
                                                        (candidateIndex + 1).coerceAtMost(
                                                            (candidates.size - 1).coerceAtLeast(0)
                                                        )
                                                candidatesOpen = true
                                                true
                                            }
                                            Key.DirectionUp -> {
                                                if (candidatesOpen)
                                                    candidateIndex =
                                                        (candidateIndex - 1).coerceAtLeast(0)
                                                candidatesOpen = true
                                                true
                                            }
                                            Key.Enter -> {
                                                val wasOpen = candidatesOpen
                                                if (wasOpen)
                                                    candidates
                                                        .getOrNull(candidateIndex)
                                                        ?.let(::addCandidate)
                                                wasOpen
                                            }
                                            Key.Escape -> {
                                                val wasOpen = candidatesOpen
                                                candidatesOpen = false
                                                wasOpen
                                            }
                                            else -> false
                                        }
                                }
                                .onGloballyPositioned { candidateAnchor = it.boundsInRoot() }
                                .pointerInput(Unit) {
                                    awaitPointerEventScope {
                                        while (true) {
                                            val event = awaitPointerEvent(PointerEventPass.Initial)
                                            if (event.changes.any { it.pressed })
                                                candidatesOpen = true
                                        }
                                    }
                                },
                    ) {
                        search = it
                        candidatesOpen = true
                    }
                    if (candidatesOpen)
                        OverlayRegistration(
                            app.t("members.candidates"),
                            { candidatesOpen = false },
                            296.dp,
                            candidateAnchor,
                            role = "listbox",
                            nonModal = true,
                            placementAbove = true,
                        ) {
                            if (candidates.isEmpty())
                                Text(
                                    app.t("members.usersEmpty"),
                                    Modifier.padding(8.dp),
                                    small = true,
                                    muted = true,
                                )
                            candidates.forEachIndexed { index, person ->
                                Row(
                                    Modifier.fillMaxWidth()
                                        .then(
                                            webControlModifier(
                                                "option",
                                                selected = index == candidateIndex,
                                            )
                                        )
                                        .clickable(
                                            enabled = "member-${person.text("id")}" !in app.pending
                                        ) {
                                            addCandidate(person)
                                        }
                                        .padding(horizontal = 8.dp, vertical = 6.dp),
                                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Avatar(person, size = 24.dp)
                                    Text(author(person), Modifier.weight(1f), maxLines = 1)
                                    if (author(person) != person.text("user_name"))
                                        Text(
                                            "@${person.text("user_name")}",
                                            small = true,
                                            muted = true,
                                            maxLines = 1,
                                        )
                                }
                            }
                        }
                }
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        app.t(
                            if (joined) if (creator) "members.creator" else "members.joined"
                            else if (channel.present("archive_at")) "moderation.archived"
                            else "members.notJoined"
                        ),
                        Modifier.weight(1f),
                        small = true,
                        muted = true,
                    )
                    if (joined)
                        Button(
                            app.t("members.leave"),
                            writable && !creator,
                            variant = "outline",
                            size = "sm",
                        ) {
                            app.run("leave-members") {
                                app.repo.members.write(
                                    app.api,
                                    "/api/channels/$id/users/${app.config.userId}",
                                    "DELETE",
                                )
                            }
                        }
                    else if (!channel.present("archive_at"))
                        Button(app.t("members.join"), writable, variant = "default", size = "sm") {
                            app.run("join-members") {
                                app.repo.members.write(
                                    app.api,
                                    "/api/channels/$id/users",
                                    "POST",
                                    obj("user_id" to app.config.userId),
                                )
                            }
                        }
                }
            }
        }
    }
    kick?.let { person ->
        Confirm(
            app.t("moderation.kickTitle"),
            app.t("moderation.kickDescription", "name" to author(person)),
            app.t("moderation.kick"),
            "kick" in app.pending,
            { kick = null },
        ) {
            app.run("kick") {
                app.repo.members.write(
                    app.api,
                    "/api/channels/$id/users/${person.text("id")}",
                    "DELETE",
                )
                kick = null
                app.showToast(app.t("moderation.kickSuccess"), ToastKind.Success)
            }
        }
    }
}

fun dmTitle(app: Session, channel: Row, users: List<Row>, memberships: List<Row>): String {
    val name = channel.text("channel_name")
    val direct =
        name.removePrefix("dm:").split(':').takeIf {
            name.startsWith("dm:") && !name.startsWith("dm:group:") && it.size == 2
        }
    val ids =
        (direct
                ?: memberships
                    .filter { it.text("channel_id") == channel.text("id") }
                    .map { it.text("user_id") })
            .filter { it != app.config.userId }
            .toSet()
    val names = users.filter { it.text("id") in ids }.map(::author).sorted()
    return names.take(3).joinToString(", ").ifEmpty { app.t("dms.untitled") } +
        if (names.size > 3) " ${app.t("dms.moreNames", "count" to (names.size - 3).toString())}"
        else ""
}

@Composable
fun MessageAdministration(app: Session, channel: Row) {
    val messages by app.repo.messages(channel.text("id")).rows.collectAsState()
    val users by app.repo.users.rows.collectAsState()
    var target by remember { mutableStateOf<Row?>(null) }
    Panel(app.t("moderation.messageAdmin"), { app.dialog = null }, 768.dp) {
        Text(app.t("moderation.messageAdminDescription"), muted = true)
        LazyColumn(Modifier.heightIn(max = 480.dp)) {
            items(
                messages
                    .filterNot { it.present("deleted_at") }
                    .sortedByDescending { timestampMillis(it.text("created_at")) },
                key = { it.text("id") },
            ) { row ->
                Row(
                    Modifier.fillMaxWidth().padding(vertical = 8.dp),
                    verticalAlignment = Alignment.Top,
                ) {
                    val user = users.find { it.text("id") == row.text("user_id") }
                    Avatar(user)
                    Column(Modifier.weight(1f).padding(start = 12.dp)) {
                        Text(user?.let(::author) ?: row.text("user_id"), weight = FontWeight.Medium)
                        RichText(row.text("content"), users)
                    }
                    IconButton(
                        "trash-2",
                        app.t(
                            "moderation.deleteMessageLabel",
                            "author" to (user?.let(::author) ?: row.text("user_id")),
                        ),
                        app.env.online,
                    ) {
                        target = row
                    }
                }
            }
        }
    }
    target?.let { row ->
        Confirm(
            app.t("moderation.forceDeleteTitle"),
            app.t("moderation.forceDeleteDescription", "author" to row.text("user_id")),
            app.t("moderation.forceDelete"),
            "force-delete" in app.pending,
            { target = null },
        ) {
            app.run("force-delete") {
                app.repo
                    .messages(channel.text("id"))
                    .write(
                        app.api,
                        "/api/messages/${row.text("id")}",
                        "DELETE",
                        null,
                        JsonObject(row + ("deleted_at" to JsonPrimitive(app.platform.now()))),
                    )
                target = null
            }
        }
    }
}
