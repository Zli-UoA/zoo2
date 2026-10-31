package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

fun JsonElement.objectOrEmpty() = this as? JsonObject ?: JsonObject(emptyMap())

fun Row.nested(key: String): JsonObject =
    (get(key) as? JsonObject)
        ?: try {
            json.parseToJsonElement(text(key)).objectOrEmpty()
        } catch (_: Exception) {
            JsonObject(emptyMap())
        }

fun Row.array(key: String): JsonArray =
    (get(key) as? JsonArray)
        ?: try {
            json.parseToJsonElement(text(key)) as? JsonArray ?: JsonArray(emptyList())
        } catch (_: Exception) {
            JsonArray(emptyList())
        }

@Composable
fun Apps(
    app: Session,
    initial: String? = null,
    close: () -> Unit = {
        app.dialog = null
        app.target = null
    },
) {
    var selected by remember(initial) { mutableStateOf(initial) }
    var revision by remember { mutableStateOf(0) }
    var editing by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    var description by remember { mutableStateOf("") }
    var icon by remember { mutableStateOf("") }
    var manifest by remember {
        mutableStateOf(
            "{\"scopes\":[\"messages:read\",\"messages:write\",\"dm:write\",\"ui:write\"],\"events\":[\"message.created\",\"app.mentioned\"],\"transport\":\"websocket\",\"dm_enabled\":true}"
        )
    }
    var credential by remember { mutableStateOf("") }
    Panel(app.t("apps.title"), close, 768.dp) {
        Text(app.t("apps.description"), muted = true)
        ApiContent("/api/apps", revision) { data, reload ->
            var detail by remember(selected) { mutableStateOf<Row?>(null) }
            val listed = data.rows().find { it.text("id") == selected }
            LaunchedEffect(selected, revision, listed) {
                detail = null
                if (selected != null && listed == null) {
                    try {
                        detail = app.api.request("/api/apps/$selected").jsonObject
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        app.toast = app.t("apps.loadError")
                    }
                }
            }
            val chosen = listed ?: detail
            Row(Modifier.heightIn(max = 520.dp)) {
                Column(
                    Modifier.width(208.dp).verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    Button(app.t("apps.create"), app.env.online) {
                        selected = null
                        editing = true
                        name = ""
                        description = ""
                        icon = ""
                    }
                    data.rows().forEach { item ->
                        Button(item.text("name")) {
                            selected = item.text("id")
                            editing = false
                            credential = ""
                        }
                    }
                }
                VerticalDivider()
                Column(
                    Modifier.weight(1f)
                        .verticalScroll(rememberScrollState())
                        .padding(start = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (editing) {
                        Field(name, app.t("apps.name")) { if (it.codePointCount() <= 80) name = it }
                        Field(description, app.t("apps.about"), singleLine = false) {
                            if (it.codePointCount() <= 4000) description = it
                        }
                        Field(icon, app.t("apps.icon")) { icon = it }
                        Field(manifest, app.t("apps.manifest"), singleLine = false) {
                            manifest = it
                        }
                        val parsed =
                            try {
                                json.parseToJsonElement(manifest) as? JsonObject
                            } catch (_: Exception) {
                                null
                            }
                        Button(
                            app.t("apps.save"),
                            name.isNotBlank() &&
                                parsed != null &&
                                (icon.isBlank() || icon.startsWith("https://")) &&
                                "app-save" !in app.pending,
                            primary = true,
                        ) {
                            app.run("app-save") {
                                val result =
                                    app.api
                                        .request(
                                            "/api/apps" + (selected?.let { "/$it" } ?: ""),
                                            if (selected == null) "POST" else "PATCH",
                                            buildJsonObject {
                                                put("name", name)
                                                put("description", description)
                                                put(
                                                    "icon_url",
                                                    icon
                                                        .takeIf { it.isNotBlank() }
                                                        ?.let(::JsonPrimitive) ?: JsonNull,
                                                )
                                                put("manifest", parsed!!)
                                            },
                                        )
                                        .jsonObject
                                selected = result.text("id")
                                editing = false
                                revision++
                            }
                        }
                    } else if (chosen != null) {
                        val id = chosen.text("id")
                        val info = chosen.nested("manifest")
                        val manage =
                            app.config.isAdmin || chosen.text("owner_id") == app.config.userId
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            if (chosen.present("icon_url"))
                                RemoteImage(
                                    chosen.text("icon_url"),
                                    chosen.text("name"),
                                    Modifier.size(32.dp),
                                )
                            Text(
                                chosen.text("name"),
                                Modifier.padding(start = 8.dp),
                                weight = FontWeight.SemiBold,
                            )
                        }
                        Text(chosen.text("description"), muted = true)
                        val people by app.repo.users.rows.collectAsState()
                        Text(
                            "${app.t("apps.developer")}: ${people.find { it.text("id") == chosen.text("owner_id") }?.let(::author) ?: app.t("apps.unknownDeveloper")}",
                            muted = true,
                            small = true,
                        )
                        Text(
                            "${app.t("apps.transport")}: ${info.text("transport")}",
                            muted = true,
                            small = true,
                        )
                        if (info.present("event_url"))
                            Text(
                                "${app.t("apps.eventEndpoint")}: ${info.text("event_url")}",
                                muted = true,
                                small = true,
                            )
                        Text(app.t("apps.status.${chosen.text("status")}"), small = true)
                        Text(app.t("apps.permissions"), weight = FontWeight.Medium)
                        info.array("scopes").forEach {
                            Text(it.jsonPrimitive.content, small = true)
                        }
                        Button(app.t("apps.revokeConsent"), app.env.online) {
                            app.run {
                                app.api.request("/api/apps/$id/authorization", "DELETE")
                                reload()
                            }
                        }
                        if (manage) {
                            Button(app.t("apps.edit")) {
                                name = chosen.text("name")
                                description = chosen.text("description")
                                icon = chosen.text("icon_url")
                                manifest =
                                    Json { prettyPrint = true }
                                        .encodeToString(JsonObject.serializer(), info)
                                editing = true
                            }
                            val actions = buildList {
                                if (chosen.text("status") == "draft")
                                    add("submit" to "requestApproval")
                                if (
                                    app.config.isAdmin &&
                                        chosen.text("status") in listOf("pending", "disabled")
                                )
                                    add("install" to "approve")
                                if (chosen.text("status") == "installed") {
                                    add("token" to "issueToken")
                                    add("connection" to "connection")
                                    add("disable" to "disable")
                                }
                            }
                            actions.forEach { (action, label) ->
                                Button(
                                    app.t("apps.$label"),
                                    app.env.online && "app:$action" !in app.pending,
                                ) {
                                    app.run("app:$action") {
                                        val result =
                                            app.api.request(
                                                "/api/apps/$id/actions/$action",
                                                "POST",
                                                if (action == "install")
                                                    buildJsonObject {
                                                        put(
                                                            "revision",
                                                            chosen["revision"] ?: JsonNull,
                                                        )
                                                    }
                                                else obj(),
                                            )
                                        if (action in listOf("token", "connection"))
                                            credential = result.toString()
                                        reload()
                                    }
                                }
                            }
                            if (app.channelId != null && chosen.text("status") == "installed")
                                Button(app.t("apps.addChannel"), app.env.online) {
                                    app.run {
                                        app.api.request(
                                            "/api/apps/$id/channels/${app.channelId}",
                                            "POST",
                                            obj(),
                                        )
                                    }
                                }
                        }
                        if (credential.isNotEmpty())
                            Credential(app.t("apps.credential"), credential)
                        if (chosen.text("status") == "installed") {
                            if (info.flag("dm_enabled"))
                                Button(app.t("apps.message"), app.env.online) {
                                    app.run {
                                        val result =
                                            app.api
                                                .request(
                                                    "/api/dms",
                                                    "POST",
                                                    buildJsonObject {
                                                        put(
                                                            "target_user_ids",
                                                            JsonArray(
                                                                listOf(
                                                                    JsonPrimitive(
                                                                        chosen.text("bot_user_id")
                                                                    )
                                                                )
                                                            ),
                                                        )
                                                    },
                                                )
                                                .jsonObject
                                        app.navigate(
                                            "/channels/${result.nested("data").text("id")}"
                                        )
                                        close()
                                    }
                                }
                            AppCommands(app, chosen)
                            Button(app.t("apps.openHome"), app.env.online) {
                                app.run {
                                    app.api.request(
                                        "/api/apps/$id/invoke",
                                        "POST",
                                        obj("kind" to "home", "channel_id" to app.channelId),
                                    )
                                }
                            }
                            AppHome(app, id)
                            if (info.flag("user_connections"))
                                ApiContent("/api/apps/$id/connection", revision) { connection, _ ->
                                    val connected =
                                        connection.objectOrEmpty().text("state") == "connected"
                                    Text(
                                        app.t(
                                            "apps.userConnection.${if (connected) "connected" else "disconnected"}"
                                        )
                                    )
                                    Button(
                                        app.t(
                                            "apps.userConnection.${if (connected) "disconnect" else "connect"}"
                                        ),
                                        app.env.online,
                                    ) {
                                        app.run {
                                            app.api.request(
                                                "/api/apps/$id/connection",
                                                "POST",
                                                buildJsonObject { put("connect", !connected) },
                                            )
                                            revision++
                                        }
                                    }
                                }
                            if (
                                info.array("scopes").any {
                                    it.jsonPrimitive.content == "assistant:write"
                                }
                            )
                                AssistantSessions(app, id)
                        }
                        if (manage) {
                            Text(app.t("apps.deliveries"), weight = FontWeight.Medium)
                            ApiContent("/api/apps/$id/deliveries", revision) { deliveries, _ ->
                                deliveries.rows().forEach { delivery ->
                                    Text(
                                        "${delivery.text("type")} · ${app.t(if (delivery.present("acked_at")) "apps.delivered" else "apps.pending")}",
                                        small = true,
                                    )
                                    if (delivery.present("last_error"))
                                        Text(
                                            delivery.text("last_error"),
                                            small = true,
                                            muted = true,
                                        )
                                    if (!delivery.present("acked_at"))
                                        Button(app.t("apps.retry"), app.env.online) {
                                            app.run {
                                                app.api.request(
                                                    "/api/apps/$id/deliveries/${delivery.text("id")}/retry",
                                                    "POST",
                                                    obj(),
                                                )
                                                revision++
                                            }
                                        }
                                }
                            }
                        }
                    } else Text(app.t("apps.choose"), muted = true)
                }
            }
        }
    }
}

@Composable
private fun AppCommands(app: Session, chosen: Row) {
    val info = chosen.nested("manifest")
    val id = chosen.text("id")
    val types =
        if (app.target != null) listOf("message_shortcuts" to "shortcut")
        else listOf("commands" to "command", "shortcuts" to "shortcut")
    types.forEach { (field, kind) ->
        info.array(field).forEach { value ->
            val name = value.jsonPrimitive.content
            var argument by remember(id, name) { mutableStateOf("") }
            if (kind == "command")
                Field(argument, "$name · ${app.t("apps.commandArguments")}") { argument = it }
            Button(name, app.env.online) {
                app.run {
                    app.api.request(
                        "/api/apps/$id/invoke",
                        "POST",
                        buildJsonObject {
                            put("kind", kind)
                            put("name", name)
                            put("channel_id", app.channelId)
                            put("message_id", app.target)
                            put("values", if (kind == "command") obj("text" to argument) else obj())
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun AppHome(app: Session, id: String) {
    var rows by remember(id) { mutableStateOf<List<Row>>(emptyList()) }
    LaunchedEffect(id, app.env.visible) {
        while (isActive && app.env.visible) {
            try {
                rows = app.api.request("/api/apps/$id/surfaces").rows()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {}
            delay(2000)
        }
    }
    rows
        .filter { it.text("kind") == "home" }
        .forEach { Blocks(app, it.nested("document")["blocks"], id, it.text("id")) }
}

@Composable
private fun AssistantSessions(app: Session, id: String) {
    var revision by remember { mutableStateOf(0) }
    var title by remember { mutableStateOf(app.t("apps.assistant.session")) }
    LaunchedEffect(id, app.channelId, app.env.online) {
        if (app.env.online && app.channelId != null) {
            try {
                app.api.request(
                    "/api/apps/$id/context",
                    "POST",
                    buildJsonObject {
                        put("channel_id", app.channelId)
                        put("context", obj("view" to "app", "channel_id" to app.channelId))
                    },
                )
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {}
        }
    }
    LaunchedEffect(id, app.env.visible, app.env.online) {
        while (isActive && app.env.visible && app.env.online) {
            delay(2000)
            revision++
        }
    }
    Field(title, app.t("apps.assistant.title")) { if (it.codePointCount() <= 200) title = it }
    Button(app.t("apps.assistant.start"), app.env.online && app.channelId != null) {
        app.run {
            app.api.request(
                "/api/assistant-sessions",
                "POST",
                buildJsonObject {
                    put("app_id", id)
                    put("user_id", app.config.userId)
                    put("channel_id", app.channelId)
                    put("title", title)
                    put("context", obj("channel_id" to app.channelId))
                },
            )
            revision++
        }
    }
    ApiContent("/api/assistant-sessions", revision) { data, _ ->
        data
            .rows()
            .filter { it.text("app_id") == id && it.text("state") == "running" }
            .forEach { session ->
                var name by remember(session.text("id")) { mutableStateOf(session.text("title")) }
                Field(name, app.t("apps.assistant.title")) {
                    if (it.codePointCount() <= 200) name = it
                }
                Row {
                    Button(app.t("apps.save"), app.env.online) {
                        app.run {
                            app.api.request(
                                "/api/assistant-sessions/${session.text("id")}",
                                "PATCH",
                                obj("title" to name),
                            )
                            revision++
                        }
                    }
                    Button(
                        app.t("apps.assistant.stop"),
                        app.env.online && session.text("state") != "stopped",
                    ) {
                        app.run {
                            app.api.request(
                                "/api/assistant-sessions/${session.text("id")}",
                                "PATCH",
                                obj("state" to "stopped"),
                            )
                            revision++
                        }
                    }
                }
            }
    }
}

fun parseBlocks(value: JsonElement?): List<Row> =
    (value as? JsonArray)
        ?.takeIf { it.size <= 50 }
        ?.mapNotNull { it as? JsonObject }
        ?.filter {
            when (it.text("type")) {
                "text" -> it["text"] is JsonPrimitive
                "button",
                "input",
                "select" ->
                    it.present("action_id") &&
                        it.present("label") &&
                        (it.text("type") != "select" ||
                            it["options"] is JsonArray &&
                                it.array("options").all { v -> v is JsonPrimitive && v.isString })
                else -> false
            }
        }
        .orEmpty()

@Composable
fun Blocks(
    app: Session,
    value: JsonElement?,
    appId: String,
    surface: String? = null,
    message: String? = null,
    channel: String? = app.channelId,
    submit: Boolean = false,
    done: () -> Unit = {},
) {
    val blocks = parseBlocks(value)
    var values by
        remember(appId, surface, message) { mutableStateOf<Map<String, String>>(emptyMap()) }
    val key = "blocks:$appId:$surface:$message"
    fun invoke(kind: String, action: String? = null, text: String? = null) {
        app.run(key) {
            app.api.request(
                "/api/apps/$appId/invoke",
                "POST",
                buildJsonObject {
                    put("kind", kind)
                    put("surface_id", surface?.let(::JsonPrimitive) ?: JsonNull)
                    put("message_id", message?.let(::JsonPrimitive) ?: JsonNull)
                    put("channel_id", channel?.let(::JsonPrimitive) ?: JsonNull)
                    put("action_id", action?.let(::JsonPrimitive) ?: JsonNull)
                    put(
                        "values",
                        buildJsonObject {
                            (values +
                                    if (action != null)
                                        mapOf(action to (text ?: values[action].orEmpty()))
                                    else emptyMap())
                                .forEach { (k, v) -> put(k, v) }
                        },
                    )
                },
            )
            if (kind == "submit") done()
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        blocks.forEach { block ->
            val id = block.text("action_id")
            when (block.text("type")) {
                "text" -> Text(block.text("text"))
                "button" ->
                    Button(block.text("label"), key !in app.pending && app.env.online) {
                        invoke("action", id, block.text("value"))
                    }
                "input" ->
                    Field(values[id].orEmpty(), block.text("label")) {
                        values = values + (id to it.take(4000))
                    }
                "select" ->
                    SelectField(
                        block.text("label"),
                        values[id].orEmpty(),
                        block.array("options").map { it.jsonPrimitive.content },
                    ) { selected ->
                        values = values + (id to selected)
                        if (!submit) invoke("action", id, selected)
                    }
            }
        }
        if (submit)
            Button(
                app.t("apps.submit"),
                key !in app.pending &&
                    app.env.online &&
                    blocks.none {
                        it.flag("required") && values[it.text("action_id")].isNullOrBlank()
                    },
                primary = true,
            ) {
                invoke("submit")
            }
    }
}

@Composable
fun AppSurfaces(app: Session) {
    fun dismiss(row: Row) {
        app.run {
            app.api.request("/api/app-surfaces/${row.text("id")}", "DELETE")
            app.surfaces = app.surfaces - row
        }
    }
    val modal = app.surfaces.find { it.text("kind") == "modal" }
    if (modal != null)
        Panel(modal.text("name"), { dismiss(modal) }) {
            Text(app.t("apps.form"), muted = true)
            Blocks(
                app,
                modal.nested("document")["blocks"],
                modal.text("app_id"),
                modal.text("id"),
                channel = modal.text("channel_id"),
                submit = true,
            ) {
                app.surfaces = app.surfaces - modal
            }
        }
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomEnd) {
        Column(
            Modifier.padding(16.dp)
                .widthIn(max = 384.dp)
                .heightIn(max = 320.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            app.surfaces
                .filter { it.text("kind") == "ephemeral" && it.text("channel_id") == app.channelId }
                .forEach { row ->
                    Surface(
                        shape = RoundedCornerShape(12.dp),
                        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                        shadowElevation = 4.dp,
                    ) {
                        Column(Modifier.padding(12.dp)) {
                            Row {
                                Text(
                                    "${row.text("name")} · ${app.t("apps.onlyYou")}",
                                    Modifier.weight(1f),
                                    small = true,
                                )
                                IconButton("x", app.t("apps.close")) { dismiss(row) }
                            }
                            Blocks(
                                app,
                                row.nested("document")["blocks"],
                                row.text("app_id"),
                                row.text("id"),
                                channel = row.text("channel_id"),
                            )
                        }
                    }
                }
        }
    }
}

@Composable
fun AppConsent(app: Session) {
    val id = app.query["app_id"].orEmpty()
    val requested = app.query["scope"].orEmpty().split(' ').filter { it.isNotEmpty() }
    var selected by remember { mutableStateOf(requested.toSet()) }
    Column(
        Modifier.fillMaxWidth().padding(24.dp).verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(app.t("apps.consentTitle"), size = 20.sp, weight = FontWeight.SemiBold)
        ApiContent("/api/apps/$id") { data, _ ->
            val item = data.objectOrEmpty()
            val info = item.nested("manifest")
            val valid =
                app.query["code_challenge_method"] == "S256" &&
                    !app.query["state"].isNullOrEmpty() &&
                    !app.query["code_challenge"].isNullOrEmpty() &&
                    info.array("redirect_uris").any {
                        it.jsonPrimitive.content == app.query["redirect_uri"]
                    } &&
                    requested.all { scope ->
                        info.array("scopes").any { it.jsonPrimitive.content == scope }
                    }
            Text(item.text("name"), weight = FontWeight.Medium)
            Text(item.text("description"))
            Text(app.t("apps.consentDescription"), muted = true)
            requested.forEach { scope ->
                Check(scope, scope in selected) {
                    selected = if (it) selected + scope else selected - scope
                }
            }
            Text(app.query["redirect_uri"].orEmpty(), small = true)
            if (!valid) Text(app.t("apps.actionError"), color = MaterialTheme.colorScheme.error)
            Row {
                Button(
                    app.t("apps.allow"),
                    valid && app.env.online && "consent" !in app.pending,
                    primary = true,
                ) {
                    app.run("consent") {
                        val result =
                            app.api
                                .request(
                                    "/api/apps/oauth/authorize",
                                    "POST",
                                    buildJsonObject {
                                        put("app_id", id)
                                        listOf("redirect_uri", "code_challenge", "state").forEach {
                                            put(it, app.query[it])
                                        }
                                        put("scopes", JsonArray(selected.map(::JsonPrimitive)))
                                    },
                                )
                                .jsonObject
                        app.platform.invoke("redirect", result.text("redirect_uri"))
                    }
                }
                Button(app.t("apps.cancel")) { app.navigate("/") }
            }
        }
    }
}

@Composable
fun Webhooks(app: Session, channel: Row) {
    val path = "/api/channels/${channel.text("id")}/webhooks"
    var name by remember { mutableStateOf("Webhook") }
    var revision by remember { mutableStateOf(0) }
    var confirmation by remember { mutableStateOf<Pair<Row, String>?>(null) }
    Panel(app.t("webhooks.title"), { app.dialog = null }, 576.dp) {
        Column(
            Modifier.verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                app.t("webhooks.description", "channel" to channel.text("channel_name")),
                muted = true,
            )
            Text(app.t("webhooks.instructions"), small = true, muted = true)
            Field(name, app.t("webhooks.name")) { if (it.codePointCount() <= 80) name = it }
            Button(
                app.t("webhooks.create"),
                app.env.online &&
                    name.isNotBlank() &&
                    !channel.present("archive_at") &&
                    "webhook" !in app.pending,
                primary = true,
            ) {
                app.run("webhook") {
                    app.api.request(path, "POST", obj("name" to name.trim()))
                    name = "Webhook"
                    revision++
                }
            }
            ApiContent(path, revision) { data, _ ->
                if (data.rows().isEmpty()) Text(app.t("webhooks.empty"), muted = true)
                data.rows().forEach { hook ->
                    HorizontalDivider()
                    Text(hook.text("name"), weight = FontWeight.Medium)
                    Credential(
                        app.t("webhooks.genericUrl"),
                        endpoint(app.config.apiBaseUrl, hook.text("path")),
                        secret = false,
                    )
                    Credential("Secret", hook.text("secret"))
                    Row {
                        Button(app.t("webhooks.rotate"), app.env.online) {
                            confirmation = hook to "rotate-secret"
                        }
                        Button(app.t("webhooks.delete"), app.env.online, destructive = true) {
                            confirmation = hook to "delete"
                        }
                    }
                    ApiContent("/api/apps") { apps, _ ->
                        apps
                            .rows()
                            .filter {
                                it.flag("can_manage") &&
                                    it.text("status") == "installed" &&
                                    it.nested("manifest").flag("notification_only")
                            }
                            .forEach { posting ->
                                Button(
                                    "${app.t("apps.changePostingApp")}: ${posting.text("name")}",
                                    app.env.online,
                                ) {
                                    app.run {
                                        app.api.request(
                                            "$path/${hook.text("id")}/app",
                                            "POST",
                                            obj("app_id" to posting.text("id")),
                                        )
                                        revision++
                                    }
                                }
                            }
                    }
                }
            }
        }
    }
    confirmation?.let { (hook, action) ->
        Confirm(
            app.t("webhooks.${if (action == "delete") "delete" else "rotate"}"),
            app.t("webhooks.${if (action == "delete") "deleteConfirm" else "rotateConfirm"}"),
            app.t("webhooks.confirm"),
            "webhook:$action" in app.pending,
            { confirmation = null },
        ) {
            app.run("webhook:$action") {
                app.api.request(
                    "$path/${hook.text("id")}" + if (action == "delete") "" else "/rotate-secret",
                    if (action == "delete") "DELETE" else "POST",
                    if (action == "delete") null else obj(),
                )
                confirmation = null
                revision++
            }
        }
    }
}

@Composable
fun MessageAppContent(app: Session, message: Row) {
    val appId = message.text("app_origin_id").ifEmpty { message.text("app_id") }
    if (appId.isNotEmpty())
        Blocks(app, message.array("app_blocks"), appId, message = message.text("id"))
    message
        .array("app_attachments")
        .rows()
        .filter { it.present("id") && it.present("name") }
        .forEach { file ->
            Button(file.text("name"), app.env.online) {
                app.run {
                    val result =
                        app.api.request("/api/app-files/${file.text("id")}/download").jsonObject
                    if (result.text("url").startsWith("https://"))
                        app.platform.action("open:${result.text("url")}")
                }
            }
        }
    message
        .array("app_unfurls")
        .rows()
        .filter { it.present("app_id") && it.present("name") }
        .forEach { unfurl ->
            Button("${unfurl.text("name")} · APP") {
                app.navigate("/apps/${unfurl.text("app_id")}")
            }
            MessageCard(unfurl.nested("card"))
            if (unfurl.present("url") && app.channelId != null)
                Row {
                    listOf("details", "comments").forEach { kind ->
                        Button(app.t("apps.entity.$kind"), app.env.online) {
                            app.run {
                                app.api.request(
                                    "/api/apps/${unfurl.text("app_id")}/entities/requests",
                                    "POST",
                                    obj(
                                        "kind" to kind,
                                        "url" to unfurl.text("url"),
                                        "message_id" to message.text("id"),
                                        "channel_id" to app.channelId,
                                    ),
                                )
                            }
                        }
                    }
                }
        }
}
