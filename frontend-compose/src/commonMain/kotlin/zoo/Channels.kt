package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.font.FontWeight
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
) {
    var name by
        remember(kind) {
            mutableStateOf(if (kind == "rename") channel?.text("channel_name").orEmpty() else "")
        }
    var description by
        remember(kind) {
            mutableStateOf(if (kind == "rename") channel?.text("description").orEmpty() else "")
        }
    var private by
        remember(kind) { mutableStateOf(kind == "rename" && channel?.flag("is_private") == true) }
    var search by remember(kind) { mutableStateOf("") }
    var selected by remember(kind) { mutableStateOf<Set<String>>(emptySet()) }
    var created by remember { mutableStateOf<String?>(null) }
    val busy = "channel-dialog" in app.pending
    fun close() {
        if (!busy) app.dialog = null
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
    Panel(app.t(title), ::close) {
        when (kind) {
            "create",
            "rename" -> {
                Field(name, app.t("channels.newName")) { name = it }
                Text(app.t("channels.editHelp"), muted = true, small = true)
                Field(description, app.t("channels.descriptionLabel"), singleLine = false) {
                    description = it
                }
                Text(
                    app.t(
                        "channels.descriptionCounter",
                        "length" to description.codePointCount().toString(),
                        "max" to "250",
                    ),
                    muted = description.codePointCount() <= 250,
                    small = true,
                )
                Check(app.t("channels.makePrivate"), private, !busy) { private = it }
                Text(app.t("channels.visibilityHelp"), muted = true, small = true)
                Row(Modifier.align(Alignment.End)) {
                    Button(app.t("common.cancel"), !busy, click = ::close)
                    Button(
                        app.t(if (kind == "create") "channels.create" else "common.save"),
                        !busy &&
                            app.env.online &&
                            validChannelName(name.trim()) &&
                            description.codePointCount() <= 250,
                        primary = true,
                    ) {
                        app.run("channel-dialog") {
                            val body = buildJsonObject {
                                put("channel_name", name.trim())
                                put(
                                    "description",
                                    description
                                        .trim()
                                        .takeIf { it.isNotEmpty() }
                                        ?.let(::JsonPrimitive) ?: JsonNull,
                                )
                                put("is_private", private)
                            }
                            app.repo.channels.write(
                                app.api,
                                if (kind == "create") "/api/channels"
                                else "/api/channels/${channel!!.text("id")}",
                                if (kind == "create") "POST" else "PATCH",
                                body,
                            )
                            app.dialog = null
                        }
                    }
                }
            }
            "browse" -> {
                Text(app.t("channels.browseDescription"), muted = true, small = true)
                Field(search, app.t("channels.browseSearch")) { search = it }
                LazyColumn(Modifier.heightIn(max = 440.dp)) {
                    val filtered =
                        visibleChannels(channels).filter {
                            !it.flag("is_dm") &&
                                !it.flag("is_private") &&
                                !it.present("archive_at") &&
                                it.text("channel_name").contains(search, true)
                        }
                    if (filtered.isEmpty())
                        item {
                            Text(
                                app.t("channels.searchEmpty"),
                                Modifier.padding(8.dp),
                                muted = true,
                            )
                        }
                    items(filtered, key = { it.text("id") }) { row ->
                        Row(
                            Modifier.fillMaxWidth().padding(vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(Modifier.weight(1f)) {
                                Text("# ${row.text("channel_name")}", weight = FontWeight.Medium)
                                Text(
                                    row.text("description"),
                                    muted = true,
                                    small = true,
                                    maxLines = 2,
                                )
                            }
                            val joined = isMember(members, row.text("id"), app.config.userId)
                            Button(
                                app.t(if (joined) "members.joined" else "channels.join"),
                                app.env.online && !busy && !joined,
                            ) {
                                app.run("channel-dialog") {
                                    app.repo.members.write(
                                        app.api,
                                        "/api/channels/${row.text("id")}/join",
                                        "POST",
                                    )
                                    app.dialog = null
                                    app.navigate("/channels/${row.text("id")}")
                                }
                            }
                        }
                    }
                }
            }
            "dm" -> {
                Text(app.t("dms.createDescription"), muted = true, small = true)
                Field(search, app.t("dms.search")) { search = it }
                LazyColumn(Modifier.heightIn(max = 350.dp)) {
                    items(
                        users.filter {
                            it.text("id") != app.config.userId &&
                                !it.flag("is_bot") &&
                                (author(it).contains(search, true) ||
                                    it.text("user_name").contains(search, true))
                        },
                        key = { it.text("id") },
                    ) { user ->
                        Check(
                            author(user),
                            user.text("id") in selected,
                            !busy && (user.text("id") in selected || selected.size < 9),
                        ) { checked ->
                            selected =
                                if (checked) selected + user.text("id")
                                else selected - user.text("id")
                        }
                    }
                }
                Text(
                    app.t("dms.selectedCount", "count" to selected.size.toString(), "max" to "9"),
                    muted = true,
                    small = true,
                )
                created?.let { id ->
                    Text(app.t("dms.syncPending"), muted = true)
                    Button(app.t("dms.openCreated")) {
                        app.dialog = null
                        app.navigate("/channels/$id")
                    }
                }
                Button(
                    app.t(if (busy) "dms.creating" else "dms.create"),
                    app.env.online && !busy && selected.isNotEmpty(),
                    primary = true,
                ) {
                    app.run("channel-dialog") {
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
                        val id = response.getValue("data").jsonObject.text("id")
                        try {
                            withTimeout(8_000) {
                                combine(app.repo.channels.rows, app.repo.members.rows) {
                                        channels,
                                        members ->
                                        channels.any { it.text("id") == id } &&
                                            isMember(members, id, app.config.userId)
                                    }
                                    .first { it }
                            }
                            app.dialog = null
                            app.navigate("/channels/$id")
                        } catch (_: TimeoutCancellationException) {
                            created = id
                        }
                    }
                }
            }
            "leave",
            "moderate" ->
                if (channel != null) {
                    val action =
                        if (kind == "leave") "leave"
                        else if (channel.present("archive_at")) "unarchive" else "archive"
                    Text(
                        app.t(
                            if (kind == "leave") "dms.leaveDescription"
                            else "moderation.${action}Description"
                        ),
                        muted = true,
                    )
                    Row(Modifier.align(Alignment.End)) {
                        Button(app.t("common.cancelShort"), !busy, click = ::close)
                        Button(
                            app.t(if (kind == "leave") "members.leave" else "moderation.execute"),
                            app.env.online && !busy,
                            destructive = true,
                        ) {
                            app.run("channel-dialog") {
                                (if (action == "leave") app.repo.members else app.repo.channels)
                                    .write(
                                        app.api,
                                        "/api/channels/${channel.text("id")}/$action",
                                        "POST",
                                    )
                                app.dialog = null
                                if (action == "leave") app.navigate("/")
                            }
                        }
                    }
                }
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
) {
    val id = channel.text("id")
    val ids = memberships.filter { it.text("channel_id") == id }.map { it.text("user_id") }.toSet()
    val manage = canManage(channel, app.config.userId, app.config.isAdmin)
    val writable = app.env.online && !channel.present("archive_at")
    val canAdd =
        writable &&
            !channel.flag("is_dm") &&
            (manage || !channel.flag("is_private") && app.config.userId in ids)
    var search by remember(id) { mutableStateOf("") }
    var kick by remember(id) { mutableStateOf<Row?>(null) }
    Column(modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (channel.present("archive_at"))
            Text(app.t("moderation.archivedMembersHelp"), muted = true, small = true)
        if (canAdd) Field(search, app.t("members.search")) { search = it }
        if (canAdd && search.isNotEmpty())
            Column(
                Modifier.fillMaxWidth()
                    .border(1.dp, MaterialTheme.colorScheme.outline)
                    .padding(4.dp)
            ) {
                val candidates =
                    users
                        .filter {
                            it.text("id") !in ids &&
                                (author(it).contains(search, true) ||
                                    it.text("user_name").contains(search, true))
                        }
                        .take(8)
                if (candidates.isEmpty())
                    Text(app.t("members.usersEmpty"), muted = true, small = true)
                candidates.forEach { user ->
                    Button(author(user), "member-${user.text("id")}" !in app.pending) {
                        app.run("member-${user.text("id")}") {
                            app.repo.members.write(
                                app.api,
                                "/api/channels/$id/users",
                                "POST",
                                obj("user_id" to user.text("id")),
                            )
                            search = ""
                        }
                    }
                }
            }
        LazyColumn(Modifier.weight(1f, fill = false)) {
            items(
                users.filter { it.text("id") in ids }.sortedBy(::author),
                key = { it.text("id") },
            ) { person ->
                Row(
                    Modifier.fillMaxWidth().padding(vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Avatar(person, Modifier.size(32.dp))
                    Column(Modifier.weight(1f).padding(start = 12.dp)) {
                        Text(author(person))
                        Text("@${person.text("user_name")}", muted = true, small = true)
                    }
                    if (person.text("id") == channel.text("created_user_id"))
                        Text(app.t("members.creator"), muted = true, small = true)
                    else if (person.text("id") == app.config.userId)
                        Text(app.t("members.you"), muted = true, small = true)
                    else if (manage && person.text("id") !in app.admins && !channel.flag("is_dm"))
                        IconButton(
                            "log-out",
                            app.t("moderation.kickLabel", "name" to author(person)),
                            writable,
                        ) {
                            kick = person
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
                app.toast = app.t("moderation.kickSuccess")
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
