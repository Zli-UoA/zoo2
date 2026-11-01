package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.*
import kotlin.math.ceil
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

@OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)
private val appPrettyJson = Json {
    prettyPrint = true
    prettyPrintIndent = "  "
}

private val defaultAppManifest =
    appPrettyJson.encodeToString(
        JsonElement.serializer(),
        json.parseToJsonElement(
            """{"scopes":["messages:read","messages:write","dm:write","ui:write"],"events":["message.created","app.mentioned"],"transport":"websocket","dm_enabled":true}"""
        ),
    )

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun Apps(
    app: Session,
    initial: String? = null,
    embedded: Boolean = false,
    initialSearch: String = "",
    initialManagement: Boolean = false,
    channelId: String? = app.contextChannelId,
    close: () -> Unit = {
        app.dialog = null
        app.target = null
    },
) {
    var search by remember(initialSearch) { mutableStateOf(initialSearch) }
    var filter by remember { mutableStateOf("all") }
    var management by remember(initialManagement) { mutableStateOf(initialManagement) }
    var selected by remember(initial) { mutableStateOf(initial) }
    var revision by remember { mutableStateOf(0) }
    var editing by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    var description by remember { mutableStateOf("") }
    var icon by remember { mutableStateOf("") }
    var manifest by remember { mutableStateOf(defaultAppManifest) }
    var confirmAction by remember { mutableStateOf<String?>(null) }
    var credential by remember { mutableStateOf("") }
    var listedData by remember { mutableStateOf<JsonElement>(JsonArray(emptyList())) }
    var loading by remember { mutableStateOf(true) }
    var loadError by remember { mutableStateOf("") }
    val allChannels by app.repo.channels.rows.collectAsState()
    val channels = allChannels.filter { !it.flag("is_dm") && !it.present("archive_at") }
    var selectedChannel by remember(channelId) { mutableStateOf(channelId) }
    LaunchedEffect(revision) {
        loading = true
        try {
            listedData = app.api.request("/api/apps")
            loadError = ""
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            loadError = app.t("apps.loadError")
        } finally {
            loading = false
        }
    }
    fun perform(key: String = "app-action", task: suspend () -> Unit) {
        app.run(key) {
            try {
                task()
                loadError = ""
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                loadError = app.t("apps.actionError")
            }
        }
    }
    val appsBusy =
        app.pending.any { it == "app-action" || it == "app-save" || it.startsWith("app:") }
    val optionLabels =
        listOf(app.t("apps.ui.chooseChannel")) + channels.map { "#${it.text("channel_name")}" }
    val textMeasurer = rememberTextMeasurer()
    val fonts = LocalFonts.current
    val textStyle = MaterialTheme.typography.bodyMedium
    val density = LocalDensity.current
    fun textAdvance(label: String): Float {
        val layout = textMeasurer.measure(fontText(label, fonts), textStyle, softWrap = false)
        return layout.getLineRight(0) - layout.getLineLeft(0)
    }
    // Native Chromium select intrinsic sizing: ceil(option advance) + padding16 + border2 +
    // arrow20.
    val selectorWidth =
        ceil(optionLabels.maxOf { with(density) { textAdvance(it).toDp().value } }.toDouble())
            .toFloat()
            .dp + 38.dp
    val channelLabel = app.t("apps.ui.channel")
    AppsContainer(app, embedded, close) {
        CssFlowRow(
            modifier = Modifier.fillMaxWidth(),
            horizontalGap = 12.dp,
            verticalGap = 12.dp,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(channelLabel, Modifier.cssPreferredWidth(textAdvance(channelLabel)))
            Box(Modifier.width(selectorWidth), propagateMinConstraints = true) {
                SelectField(
                    channelLabel,
                    channels
                        .find { it.text("id") == selectedChannel }
                        ?.let { "#${it.text("channel_name")}" } ?: app.t("apps.ui.chooseChannel"),
                    optionLabels,
                    allowEmpty = false,
                ) { label ->
                    selectedChannel =
                        channels.find { "#${it.text("channel_name")}" == label }?.text("id")
                }
            }
        }
        if (loadError.isNotEmpty())
            Text(
                loadError,
                Modifier.fillMaxWidth()
                    .background(
                        MaterialTheme.colorScheme.error.copy(alpha = .05f),
                        RoundedCornerShape(8.dp),
                    )
                    .border(
                        1.dp,
                        MaterialTheme.colorScheme.error.copy(alpha = .3f),
                        RoundedCornerShape(8.dp),
                    )
                    .padding(13.dp)
                    .then(webControlModifier("alert")),
                color = MaterialTheme.colorScheme.error,
            )
        run {
            val data = listedData
            val reload = { revision++ }
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
            BoxWithConstraints(Modifier.fillMaxWidth()) {
                val wide = LocalViewportWidth.current >= 768f
                val boundedHeight = constraints.hasBoundedHeight
                val sidebarScroll = rememberScrollState()
                val detailScroll = rememberScrollState()
                val density = LocalDensity.current
                val detailMaxHeight =
                    with(density) { LocalWindowInfo.current.containerSize.height.toDp() } * .65f
                var sidebarHeight by remember { mutableStateOf(0.dp) }
                var detailHeight by remember { mutableStateOf(0.dp) }
                Row(Modifier.fillMaxWidth()) {
                    if (wide || selected == null && !editing)
                        AppSidebarColumn(
                            (if (wide) Modifier.width(223.dp).padding(end = 16.dp)
                                else Modifier.weight(1f))
                                .onSizeChanged {
                                    sidebarHeight = with(density) { it.height.toDp() }
                                }
                                .then(
                                    if (!embedded && boundedHeight)
                                        Modifier.verticalScroll(sidebarScroll)
                                    else Modifier
                                ),
                            minimumHeight = if (wide) detailHeight else 0.dp,
                        ) {
                            Column(
                                Modifier.fillMaxWidth(),
                                verticalArrangement = Arrangement.spacedBy(12.dp),
                            ) {
                                Field(
                                    search,
                                    app.t("apps.ui.search"),
                                    showLabel = false,
                                    placeholder = app.t("apps.ui.search"),
                                    leadingIcon = "search",
                                ) {
                                    search = it
                                }
                                val filters =
                                    listOf("all", "installed", "mine") +
                                        if (app.config.isAdmin) listOf("pending") else emptyList()
                                Box(Modifier.fillMaxWidth(), propagateMinConstraints = true) {
                                    SelectField(
                                        app.t("apps.ui.filter"),
                                        app.t(
                                            if (filter == "all" || filter == "mine")
                                                "apps.ui.$filter"
                                            else "apps.status.$filter"
                                        ),
                                        filters.map {
                                            app.t(
                                                if (it == "all" || it == "mine") "apps.ui.$it"
                                                else "apps.status.$it"
                                            )
                                        },
                                        allowEmpty = false,
                                    ) { label ->
                                        filter =
                                            filters.first {
                                                app.t(
                                                    if (it == "all" || it == "mine") "apps.ui.$it"
                                                    else "apps.status.$it"
                                                ) == label
                                            }
                                    }
                                }
                                val visible =
                                    data.rows().filter { item ->
                                        item.text("name").contains(search, true) &&
                                            (filter == "all" ||
                                                filter == "mine" &&
                                                    item.text("owner_id") == app.config.userId ||
                                                item.text("status") == filter)
                                    }
                                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    if (loading)
                                        Text(
                                            app.t("apps.loading"),
                                            Modifier.padding(12.dp)
                                                .then(webControlModifier("status")),
                                            muted = true,
                                        )
                                    if (!loading && visible.isEmpty())
                                        Text(
                                            app.t("apps.ui.empty"),
                                            Modifier.padding(12.dp),
                                            muted = true,
                                        )
                                    visible.forEach { item ->
                                        Row(
                                            Modifier.fillMaxWidth()
                                                .then(
                                                    webControlModifier(
                                                        "button",
                                                        selected = selected == item.text("id"),
                                                        enabled = !appsBusy,
                                                    )
                                                )
                                                .background(
                                                    if (selected == item.text("id"))
                                                        MaterialTheme.colorScheme.secondaryContainer
                                                    else
                                                        androidx.compose.ui.graphics.Color
                                                            .Transparent,
                                                    RoundedCornerShape(10.dp),
                                                )
                                                .clickable(enabled = !appsBusy) {
                                                    selected = item.text("id")
                                                    editing = false
                                                    credential = ""
                                                    management = false
                                                }
                                                .padding(12.dp),
                                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                                            verticalAlignment = Alignment.CenterVertically,
                                        ) {
                                            Avatar(
                                                obj(
                                                    "display_name" to item.text("name"),
                                                    "is_bot" to true,
                                                    "avatar_url" to item.text("icon_url"),
                                                ),
                                                size = 24.dp,
                                            )
                                            Column {
                                                Text(item.text("name"), weight = FontWeight.Medium)
                                                Text(
                                                    app.t("apps.status.${item.text("status")}"),
                                                    small = true,
                                                    muted = true,
                                                )
                                            }
                                        }
                                    }
                                }
                                if (loadError.isNotEmpty() && !loading)
                                    Button(
                                        app.t("offline.retry"),
                                        app.env.online && !appsBusy,
                                        variant = "outline",
                                        modifier = Modifier.fillMaxWidth(),
                                    ) {
                                        loadError = ""
                                        revision++
                                    }
                            }
                            Button(
                                app.t("apps.create"),
                                app.env.online && !appsBusy,
                                variant = "outline",
                                modifier = Modifier.padding(top = 12.dp).fillMaxWidth(),
                                icon = "plus",
                            ) {
                                selected = null
                                editing = true
                                name = ""
                                description = ""
                                icon = ""
                                manifest = defaultAppManifest
                                credential = ""
                                loadError = ""
                                management = false
                            }
                        }
                    if (wide) VerticalDivider(Modifier.height(maxOf(sidebarHeight, detailHeight)))
                    if (wide || selected != null || editing)
                        Column(
                            Modifier.weight(1f)
                                .onSizeChanged { detailHeight = with(density) { it.height.toDp() } }
                                .then(
                                    if (!embedded)
                                        Modifier.heightIn(max = detailMaxHeight)
                                            .verticalScroll(detailScroll)
                                    else Modifier
                                )
                                .padding(start = if (wide) 16.dp else 0.dp),
                            verticalArrangement = Arrangement.spacedBy(16.dp),
                        ) {
                            if (!wide)
                                Button(app.t("apps.ui.back"), size = "sm", icon = "arrow-left") {
                                    selected = null
                                    editing = false
                                    credential = ""
                                }
                            if (editing) {
                                Text(
                                    app.t(if (selected != null) "apps.edit" else "apps.create"),
                                    size = 16.sp,
                                    lineHeight = 24.sp,
                                    weight = FontWeight.SemiBold,
                                )
                                Field(name, app.t("apps.name")) {
                                    if (it.codePointCount() <= 80) name = it
                                }
                                Field(description, app.t("apps.about"), singleLine = false) {
                                    if (it.codePointCount() <= 4000) description = it
                                }
                                Field(icon, app.t("apps.icon")) { icon = it }
                                Field(
                                    manifest,
                                    app.t("apps.manifest"),
                                    singleLine = false,
                                    minimumHeight = 240.dp,
                                    inputTextStyle =
                                        TextStyle(
                                            fontFamily = FontFamily.Monospace,
                                            fontSize = 12.sp,
                                            lineHeight = 16.sp,
                                        ),
                                ) {
                                    manifest = it
                                }
                                val parsed =
                                    try {
                                        json.parseToJsonElement(manifest) as? JsonObject
                                    } catch (_: Exception) {
                                        null
                                    }
                                CssFlowRow(
                                    Modifier.fillMaxWidth(),
                                    horizontalGap = 8.dp,
                                    verticalGap = 8.dp,
                                    alignEnd = true,
                                ) {
                                    Button(app.t("common.cancel"), !appsBusy, variant = "outline") {
                                        editing = false
                                        loadError = ""
                                    }
                                    Button(
                                        app.t("apps.save"),
                                        name.isNotBlank() &&
                                            (icon.isBlank() || publicUrl(icon) != null) &&
                                            "app-save" !in app.pending,
                                        primary = true,
                                    ) {
                                        if (parsed == null)
                                            loadError = app.t("apps.ui.invalidManifest")
                                        else
                                            perform("app-save") {
                                                val result =
                                                    app.api
                                                        .request(
                                                            "/api/apps" +
                                                                (selected?.let { "/$it" } ?: ""),
                                                            if (selected == null) "POST"
                                                            else "PATCH",
                                                            buildJsonObject {
                                                                put("name", name)
                                                                put("description", description)
                                                                put(
                                                                    "icon_url",
                                                                    icon
                                                                        .takeIf { it.isNotBlank() }
                                                                        ?.let(::JsonPrimitive)
                                                                        ?: JsonNull,
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
                                }
                            } else if (chosen != null) {
                                val id = chosen.text("id")
                                val info = chosen.nested("manifest")
                                val manage =
                                    app.config.isAdmin ||
                                        chosen.text("owner_id") == app.config.userId
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                                ) {
                                    Avatar(
                                        obj(
                                            "display_name" to chosen.text("name"),
                                            "is_bot" to true,
                                            "avatar_url" to chosen.text("icon_url"),
                                        )
                                    )
                                    Column {
                                        Row(
                                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                                            verticalAlignment = Alignment.CenterVertically,
                                        ) {
                                            Text(
                                                chosen.text("name"),
                                                Modifier.alignByBaseline(),
                                                size = 16.sp,
                                                lineHeight = 24.sp,
                                                weight = FontWeight.SemiBold,
                                            )
                                            Text(
                                                "APP",
                                                Modifier.alignByBaseline(),
                                                small = true,
                                                muted = true,
                                                weight = FontWeight.SemiBold,
                                            )
                                        }
                                        Text(
                                            app.t("apps.status.${chosen.text("status")}"),
                                            small = true,
                                            muted = true,
                                        )
                                    }
                                }
                                Text(chosen.text("description"))
                                val people by app.repo.users.rows.collectAsState()
                                Row(
                                    Modifier.fillMaxWidth()
                                        .background(
                                            MaterialTheme.colorScheme.surfaceVariant,
                                            RoundedCornerShape(10.dp),
                                        )
                                        .padding(4.dp),
                                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                                ) {
                                    Button(
                                        app.t("apps.ui.overview"),
                                        role = androidx.compose.ui.semantics.Role.Tab,
                                        selected = !management,
                                        variant = "tab",
                                    ) {
                                        management = false
                                    }
                                    if (manage)
                                        Button(
                                            app.t("apps.ui.manage"),
                                            role = androidx.compose.ui.semantics.Role.Tab,
                                            selected = management,
                                            variant = "tab",
                                        ) {
                                            management = true
                                        }
                                }
                                if (manage && management)
                                    SettingsSection(app.t("apps.ui.configuration")) {
                                        FlowRow(
                                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                                            verticalArrangement = Arrangement.spacedBy(8.dp),
                                        ) {
                                            Button(
                                                app.t("apps.edit"),
                                                !appsBusy,
                                                variant = "outline",
                                            ) {
                                                name = chosen.text("name")
                                                description = chosen.text("description")
                                                icon = chosen.text("icon_url")
                                                manifest =
                                                    appPrettyJson.encodeToString(
                                                        JsonObject.serializer(),
                                                        info,
                                                    )
                                                editing = true
                                            }
                                            val actions = buildList {
                                                if (
                                                    chosen.text("status") in
                                                        listOf("draft", "disabled")
                                                )
                                                    add("submit" to "requestApproval")
                                                if (
                                                    app.config.isAdmin &&
                                                        chosen.text("status") == "pending"
                                                )
                                                    add("install" to "approve")
                                                if (chosen.text("status") == "installed") {
                                                    add("token" to "issueToken")
                                                    add("connection" to "connection")
                                                }
                                                if (chosen.text("status") != "disabled")
                                                    add("disable" to "disable")
                                            }
                                            actions.forEach { (action, label) ->
                                                Button(
                                                    app.t("apps.$label"),
                                                    app.env.online &&
                                                        !appsBusy &&
                                                        "app:$action" !in app.pending,
                                                    variant =
                                                        if (action == "disable") "destructive"
                                                        else if (action == "install") "default"
                                                        else "outline",
                                                ) {
                                                    if (action == "disable")
                                                        confirmAction = "disable"
                                                    else
                                                        perform("app:$action") {
                                                            val result =
                                                                app.api.request(
                                                                    "/api/apps/$id/actions/$action",
                                                                    "POST",
                                                                    if (action == "install")
                                                                        buildJsonObject {
                                                                            put(
                                                                                "revision",
                                                                                chosen["revision"]
                                                                                    ?: JsonNull,
                                                                            )
                                                                        }
                                                                    else obj(),
                                                                )
                                                            if (
                                                                action in
                                                                    listOf("token", "connection")
                                                            )
                                                                credential =
                                                                    appPrettyJson.encodeToString(
                                                                        JsonElement.serializer(),
                                                                        result,
                                                                    )
                                                            reload()
                                                        }
                                                }
                                            }
                                        }
                                    }
                                if (manage && management)
                                    SettingsSection(
                                        app.t("apps.ui.delete"),
                                        app.t("apps.ui.deleteHelp"),
                                        borderColor =
                                            MaterialTheme.colorScheme.error.copy(alpha = .3f),
                                    ) {
                                        Button(
                                            app.t("apps.ui.delete"),
                                            !appsBusy,
                                            variant = "destructive",
                                        ) {
                                            confirmAction = "delete"
                                        }
                                    }
                                if (management && credential.isNotEmpty())
                                    AppCredential(app, credential)
                                if (management && manage)
                                    SettingsSection(app.t("apps.connection")) {
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
                                    }
                                if (manage && management) AppPermissions(app, info.array("scopes"))
                                if (!management && chosen.text("status") == "installed") {
                                    AppConnections(
                                        app,
                                        id,
                                        selectedChannel,
                                        info.flag("user_connections"),
                                        info.array("scopes").any {
                                            it.jsonPrimitive.content == "assistant:write"
                                        },
                                    )
                                    Text(
                                        "${app.t("apps.developer")}: ${people.find {it.text("id") == chosen.text("owner_id")}?.text("display_name").orEmpty().ifEmpty {app.t("apps.unknownDeveloper")}}",
                                        small = true,
                                        muted = true,
                                    )
                                    CssFlowRow(horizontalGap = 8.dp, verticalGap = 8.dp) {
                                        if (
                                            info.flag("dm_enabled") &&
                                                !info.flag("notification_only")
                                        )
                                            Button(
                                                app.t("apps.message"),
                                                app.env.online &&
                                                    !appsBusy &&
                                                    "app-action" !in app.pending,
                                                primary = true,
                                            ) {
                                                perform {
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
                                                                                    chosen.text(
                                                                                        "bot_user_id"
                                                                                    )
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
                                        if (
                                            selectedChannel != null &&
                                                !info.flag("notification_only")
                                        )
                                            Button(
                                                app.t("apps.addChannel"),
                                                app.env.online && !appsBusy,
                                                variant = "outline",
                                            ) {
                                                perform {
                                                    app.api.request(
                                                        "/api/apps/$id/channels/$selectedChannel",
                                                        "POST",
                                                        obj(),
                                                    )
                                                }
                                            }
                                    }
                                    AppCommands(
                                        app,
                                        chosen,
                                        selectedChannel,
                                        busy = appsBusy,
                                        revoke = {
                                            perform {
                                                app.api.request(
                                                    "/api/apps/$id/authorization",
                                                    "DELETE",
                                                )
                                                reload()
                                            }
                                        },
                                        invoke = { kind, action, argument ->
                                            perform {
                                                app.api.request(
                                                    "/api/apps/$id/invoke",
                                                    "POST",
                                                    buildJsonObject {
                                                        put("kind", kind)
                                                        action?.let { put("name", it) }
                                                        put(
                                                            "values",
                                                            if (kind == "command")
                                                                obj("text" to argument.orEmpty())
                                                            else obj(),
                                                        )
                                                        if (kind == "shortcut")
                                                            app.target?.let {
                                                                put("message_id", it)
                                                            }
                                                        put(
                                                            "channel_id",
                                                            if (kind == "home") JsonNull
                                                            else
                                                                selectedChannel?.let(
                                                                    ::JsonPrimitive
                                                                ) ?: JsonNull,
                                                        )
                                                    },
                                                )
                                            }
                                        },
                                    )
                                    AppHome(app, id, selectedChannel)

                                    AppPermissions(app, info.array("scopes"))
                                }
                                if (!management && chosen.text("status") != "installed") {
                                    Text(
                                        "${app.t("apps.developer")}: ${people.find { it.text("id") == chosen.text("owner_id") }?.text("display_name").orEmpty().ifEmpty { app.t("apps.unknownDeveloper") }}",
                                        small = true,
                                        muted = true,
                                    )
                                    Button(
                                        app.t("apps.revokeConsent"),
                                        app.env.online && !appsBusy,
                                        variant = "ghost",
                                    ) {
                                        perform {
                                            app.api.request("/api/apps/$id/authorization", "DELETE")
                                            reload()
                                        }
                                    }
                                    AppPermissions(app, info.array("scopes"))
                                }
                                if (manage && management) {
                                    var deliveries by
                                        remember(id) { mutableStateOf<List<Row>>(emptyList()) }
                                    SettingsSection(app.t("apps.deliveries")) {
                                        Button(app.t("apps.deliveries"), variant = "ghost") {
                                            perform {
                                                deliveries =
                                                    app.api
                                                        .request("/api/apps/$id/deliveries")
                                                        .rows()
                                            }
                                        }
                                        deliveries.forEach { delivery ->
                                            Column(
                                                Modifier.fillMaxWidth()
                                                    .border(
                                                        1.dp,
                                                        MaterialTheme.colorScheme.outline,
                                                        RoundedCornerShape(4.dp),
                                                    )
                                                    .padding(8.dp)
                                            ) {
                                                Text(
                                                    "${delivery.text("type")} · ${delivery.text("attempts")} · ${if (delivery.present("acked_at")) app.t("apps.delivered") else delivery.text("last_error").ifEmpty { app.t("apps.pending") }}",
                                                    small = true,
                                                )
                                                if (!delivery.present("acked_at"))
                                                    Button(
                                                        app.t("apps.retry"),
                                                        app.env.online && !appsBusy,
                                                        variant = "ghost",
                                                        size = "sm",
                                                    ) {
                                                        perform {
                                                            app.api.request(
                                                                "/api/apps/$id/deliveries/${delivery.text("id")}/retry",
                                                                "POST",
                                                                obj(),
                                                            )
                                                        }
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
    confirmAction?.let { action ->
        ConfirmationPanel(
            app.t(
                if (action == "delete") "apps.ui.deleteTitle" else "apps.disable",
                "name" to (listedData.rows().find { it.text("id") == selected }?.text("name") ?: ""),
            ),
            app.t(if (action == "delete") "apps.ui.deleteHelp" else "apps.ui.disableHelp"),
            app.t(if (action == "delete") "apps.ui.delete" else "apps.disable"),
            "app:$action" in app.pending,
            loadError.takeIf { it.isNotEmpty() },
            {
                confirmAction = null
                loadError = ""
            },
        ) {
            perform("app:$action") {
                app.api.request("/api/apps/$selected/actions/$action", "POST", obj())
                if (action == "delete") {
                    selected = null
                    editing = false
                    credential = ""
                }
                confirmAction = null
                revision++
            }
        }
    }
}

@Composable
private fun AppCommands(
    app: Session,
    chosen: Row,
    channelId: String?,
    busy: Boolean,
    revoke: () -> Unit,
    invoke: (String, String?, String?) -> Unit,
) {
    val info = chosen.nested("manifest")
    val commands = if (app.target == null) info.array("commands") else JsonArray(emptyList())
    val enabled = app.env.online && !busy
    if (commands.isNotEmpty())
        Button(app.t("apps.revokeConsent"), enabled, variant = "ghost", click = revoke)
    commands.forEach { value ->
        val name = value.jsonPrimitive.content
        var argument by remember(chosen.text("id"), name) { mutableStateOf("") }
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Bottom,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text("/$name ${app.t("apps.commandArguments")}")
                Field(argument, "/$name ${app.t("apps.commandArguments")}", showLabel = false) {
                    argument = it.take(4000)
                }
            }
            Button("/$name", enabled && channelId != null, variant = "outline") {
                invoke("command", name, argument)
            }
        }
    }
    // HTML inline-flex buttons share one formatting line between block command rows.
    CssFlowRow {
        if (commands.isEmpty())
            Button(app.t("apps.revokeConsent"), enabled, variant = "ghost", click = revoke)
        info.array(if (app.target != null) "message_shortcuts" else "shortcuts").forEach { value ->
            val shortcut = value.jsonPrimitive.content
            Button(shortcut, enabled, variant = "outline") { invoke("shortcut", shortcut, null) }
        }
        Button(app.t("apps.openHome"), enabled, variant = "ghost") { invoke("home", null, null) }
    }
}

@Composable
private fun AppHome(app: Session, id: String, channelId: String? = app.contextChannelId) {
    var rows by remember(id) { mutableStateOf<List<Row>>(emptyList()) }
    LaunchedEffect(id, app.env.visible) {
        while (isActive) {
            try {
                rows = app.api.request("/api/apps/$id/surfaces").rows()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {}
            if (!app.env.visible) break
            delay(2000)
        }
    }
    rows
        .filter { it.text("kind") == "home" }
        .forEach {
            Column(
                Modifier.fillMaxWidth()
                    .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(10.dp))
                    .padding(12.dp)
            ) {
                Blocks(
                    app,
                    it.nested("document")["blocks"],
                    id,
                    it.text("id"),
                    channel = it.text("channel_id").ifEmpty { channelId },
                )
            }
        }
}

fun parseBlocks(value: JsonElement?): List<Row> =
    ((if (value is JsonPrimitive)
            try {
                json.parseToJsonElement(value.content)
            } catch (_: Exception) {
                JsonNull
            }
        else value)
            as? JsonArray)
        ?.takeIf { it.size <= 50 }
        ?.mapNotNull { it as? JsonObject }
        ?.filter {
            when (it.text("type")) {
                "card" ->
                    (it["card"] as? JsonObject)?.text("title")?.isNotBlank() == true &&
                        messageCards(it["card"])?.size == 1
                "text" -> (it["text"] as? JsonPrimitive)?.isString == true
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
    channel: String? = null,
    submit: Boolean = false,
    done: suspend () -> Unit = {},
) {
    val blocks = parseBlocks(value)
    var values by
        remember(appId, surface, message) { mutableStateOf<Map<String, String>>(emptyMap()) }
    val key = "blocks:$appId:$surface:$message"
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    fun invoke(kind: String, action: String? = null, text: String? = null) {
        if (busy) return
        app.scope.launch {
            busy = true
            error = ""
            try {
                app.api.request(
                    "/api/apps/$appId/invoke",
                    "POST",
                    buildJsonObject {
                        put("kind", kind)
                        if (surface != null) put("surface_id", surface)
                        if (message != null) put("message_id", message)
                        if (channel != null) put("channel_id", channel)
                        if (action != null) put("action_id", action)
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
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                error = app.t("apps.actionError")
            } finally {
                busy = false
            }
        }
    }
    Column(
        Modifier.fillMaxWidth()
            .padding(
                top = if (blocks.firstOrNull()?.text("type") == "card") 0.dp else 8.dp,
                bottom = if (blocks.lastOrNull()?.text("type") == "card") 0.dp else 8.dp,
            ),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        blocks.forEach { block ->
            val id = block.text("action_id")
            when (block.text("type")) {
                "card" -> MessageCard(block.nested("card"), attachment = true)
                "text" -> Text(block.text("text"))
                "button" ->
                    Button(
                        block.text("label"),
                        !busy && app.env.online,
                        variant = "outline",
                        size = "sm",
                    ) {
                        invoke("action", id, block.text("value"))
                    }
                "input" ->
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(block.text("label"))
                        Field(values[id].orEmpty(), block.text("label"), showLabel = false) {
                            values = values + (id to it.take(4000))
                        }
                    }
                "select" ->
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(block.text("label"))
                        Box(Modifier.fillMaxWidth(), propagateMinConstraints = true) {
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
            }
        }
        if (submit)
            Button(app.t("apps.submit"), !busy && app.env.online, primary = true) {
                if (
                    blocks.none {
                        it.flag("required") && values[it.text("action_id")].isNullOrBlank()
                    }
                )
                    invoke("submit")
            }
        if (error.isNotEmpty())
            Text(
                error,
                Modifier.then(webControlModifier("alert")).semantics {
                    liveRegion = androidx.compose.ui.semantics.LiveRegionMode.Polite
                },
                color = MaterialTheme.colorScheme.error,
            )
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
                .filter {
                    it.text("kind") == "ephemeral" && it.text("channel_id") == app.contextChannelId
                }
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
    val requested =
        remember(app.env.path) {
            app.query["scope"].orEmpty().split(' ').filter { it.isNotEmpty() }
        }
    var selected by remember { mutableStateOf(requested.toSet()) }
    var item by remember(id) { mutableStateOf<Row?>(null) }
    var error by remember(id) { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    val people by app.repo.users.rows.collectAsState()
    LaunchedEffect(id) {
        if (id.isEmpty()) error = app.t("apps.actionError")
        else
            try {
                item = app.api.request("/api/apps/$id").jsonObject
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                error = app.t("apps.loadError")
            }
    }
    val info = item?.nested("manifest") ?: obj()
    val valid =
        item != null &&
            app.query["code_challenge_method"] == "S256" &&
            !app.query["state"].isNullOrEmpty() &&
            !app.query["code_challenge"].isNullOrEmpty() &&
            info.array("redirect_uris").any {
                it.jsonPrimitive.content == app.query["redirect_uri"]
            } &&
            requested.all { scope ->
                info.array("scopes").any { it.jsonPrimitive.content == scope }
            }
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
        Column(
            Modifier.widthIn(max = 512.dp)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                app.t("apps.consentTitle"),
                size = 20.sp,
                lineHeight = 28.sp,
                weight = FontWeight.SemiBold,
            )
            item?.let { data ->
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Avatar(
                        obj(
                            "display_name" to data.text("name"),
                            "is_bot" to true,
                            "avatar_url" to data.text("icon_url"),
                        )
                    )
                    Text(
                        data.text("name"),
                        Modifier.semantics { heading() },
                        size = 16.sp,
                        lineHeight = 24.sp,
                    )
                }
                Text(data.text("description"), size = 16.sp, lineHeight = 24.sp)
                Text(
                    "${app.t("apps.developer")}: ${people.find {it.text("id") == data.text("owner_id")}?.text("display_name").orEmpty().ifEmpty {app.t("apps.unknownDeveloper")}}",
                    small = true,
                )
                Text(app.t("apps.consentDescription"))
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    requested.forEach { scope ->
                        Check(scope, scope in selected) {
                            selected = if (it) selected + scope else selected - scope
                        }
                    }
                }
                Text(app.query["redirect_uri"].orEmpty(), small = true)
            }
            if (error.isNotEmpty() || item != null && !valid)
                Text(
                    error.ifEmpty { app.t("apps.actionError") },
                    Modifier.then(webControlModifier("alert")),
                    color = MaterialTheme.colorScheme.error,
                )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(app.t("apps.allow"), valid && !busy, primary = true) {
                    if (!busy)
                        app.scope.launch {
                            busy = true
                            try {
                                val result =
                                    app.api
                                        .request(
                                            "/api/apps/oauth/authorize",
                                            "POST",
                                            buildJsonObject {
                                                put("app_id", id)
                                                listOf("redirect_uri", "code_challenge", "state")
                                                    .forEach { put(it, app.query[it]) }
                                                put(
                                                    "scopes",
                                                    JsonArray(selected.map(::JsonPrimitive)),
                                                )
                                            },
                                        )
                                        .jsonObject
                                app.platform.invoke("redirect", result.text("redirect_uri"))
                            } catch (e: CancellationException) {
                                throw e
                            } catch (_: Exception) {
                                error = app.t("apps.actionError")
                                busy = false
                            }
                        }
                }
                Button(app.t("apps.cancel"), variant = "outline") { app.navigate("/") }
            }
        }
    }
}

private fun appApiMessage(app: Session, error: Exception): String =
    when ((error as? ApiException)?.status) {
        403 -> app.t("moderation.forbidden")
        404 -> app.t("moderation.notFound")
        400 -> app.t("moderation.invalidState")
        else ->
            (error as? ApiException)?.body?.text("error").orEmpty().ifEmpty {
                error.message ?: app.t("apps.actionError")
            }
    }

@Composable
private fun WebhookCredential(app: Session, label: String, value: String) {
    var visible by remember(value) { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(label, small = true)
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Field(
                value,
                label,
                Modifier.weight(1f),
                secret = !visible,
                inputTextStyle =
                    TextStyle(
                        fontFamily = FontFamily.Monospace,
                        fontSize = 12.sp,
                        lineHeight = 16.sp,
                    ),
                readOnly = true,
                showLabel = false,
            ) {}
            Button(
                app.t(if (visible) "webhooks.hideUrl" else "webhooks.showUrl"),
                variant = "ghost",
                size = "icon-sm",
                icon = if (visible) "eye-off" else "eye",
            ) {
                visible = !visible
            }
            Button(
                app.t("webhooks.copyLabel", "label" to label),
                variant = "outline",
                size = "icon-sm",
                icon = "copy",
            ) {
                app.run {
                    try {
                        app.platform.invoke("copy", value)
                        app.showToast(app.t("webhooks.copied"), ToastKind.Success)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        app.toast = app.t("webhooks.copyError")
                    }
                }
            }
        }
    }
}

@Composable
private fun WebhookAppSelect(app: Session, channelId: String, hook: Row, updated: () -> Unit) {
    var apps by remember { mutableStateOf<List<Row>?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (apps == null)
            Button(app.t("apps.changePostingApp"), !busy, variant = "ghost", size = "sm") {
                app.scope.launch {
                    busy = true
                    try {
                        apps =
                            app.api.request("/api/apps").rows().filter {
                                it.flag("can_manage") &&
                                    it.text("status") == "installed" &&
                                    it.nested("manifest").flag("notification_only")
                            }
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        error = true
                    } finally {
                        busy = false
                    }
                }
            }
        else
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(app.t("apps.postingApp"), small = true)
                val list = apps.orEmpty()
                Box(Modifier.fillMaxWidth(), propagateMinConstraints = true) {
                    SelectField(
                        app.t("apps.postingApp"),
                        list
                            .find { it.text("bot_user_id") == hook.text("bot_user_id") }
                            ?.text("name") ?: app.t("apps.choosePostingApp"),
                        listOf(app.t("apps.choosePostingApp")) + list.map { it.text("name") },
                        enabled = !busy,
                        allowEmpty = false,
                    ) { name ->
                        list
                            .find { it.text("name") == name }
                            ?.let { chosen ->
                                app.scope.launch {
                                    busy = true
                                    try {
                                        app.api.request(
                                            "/api/channels/$channelId/webhooks/${hook.text("id")}/app",
                                            "POST",
                                            obj("app_id" to chosen.text("id")),
                                        )
                                        updated()
                                    } catch (e: CancellationException) {
                                        throw e
                                    } catch (_: Exception) {
                                        error = true
                                    } finally {
                                        busy = false
                                    }
                                }
                            }
                    }
                }
            }
        if (error)
            Text(
                app.t("apps.actionError"),
                Modifier.then(webControlModifier("alert")),
                small = true,
                color = MaterialTheme.colorScheme.error,
            )
    }
}

@Composable
fun Webhooks(app: Session, channel: Row, close: () -> Unit = { app.dialog = null }) {
    val path = "/api/channels/${channel.text("id")}/webhooks"
    var name by remember { mutableStateOf("Webhook") }
    var revision by remember { mutableStateOf(0) }
    var hooks by remember(path) { mutableStateOf<List<Row>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var confirmation by remember { mutableStateOf<Pair<Row, String>?>(null) }
    LaunchedEffect(path, revision) {
        loading = true
        error = ""
        try {
            hooks = app.api.request(path).rows()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = appApiMessage(app, e)
        } finally {
            loading = false
        }
    }
    fun run(task: suspend () -> Unit) {
        if (!busy)
            app.scope.launch {
                busy = true
                error = ""
                try {
                    task()
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    error = appApiMessage(app, e)
                } finally {
                    busy = false
                }
            }
    }
    Panel(
        app.t("webhooks.title"),
        close,
        576.dp,
        description = app.t("webhooks.description", "channel" to channel.text("channel_name")),
    ) {
        Column(
            Modifier.verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(app.t("webhooks.instructions"), small = true, muted = true)
            if (error.isNotEmpty())
                Row(
                    Modifier.then(webControlModifier("alert")),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(error, color = MaterialTheme.colorScheme.error)
                    Button(
                        app.t("webhooks.retry"),
                        !busy && !loading,
                        variant = "ghost",
                        size = "sm",
                    ) {
                        revision++
                    }
                }
            if (loading)
                Text(app.t("webhooks.loading"), Modifier.then(webControlModifier("status")))
            else {
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.Bottom,
                ) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(app.t("webhooks.name"))
                        Field(
                            name,
                            app.t("webhooks.name"),
                            showLabel = false,
                            enabled = !busy && !channel.present("archive_at"),
                        ) {
                            if (it.codePointCount() <= 80) name = it
                        }
                    }
                    Button(
                        app.t("webhooks.create"),
                        !busy && name.isNotBlank() && !channel.present("archive_at"),
                        primary = true,
                    ) {
                        run {
                            val response =
                                app.api.request(path, "POST", obj("name" to name.trim())).jsonObject
                            val created = response.nested("data").ifEmpty { response }
                            hooks = hooks + created
                            name = "Webhook"
                        }
                    }
                }
                if (channel.present("archive_at"))
                    Text(app.t("webhooks.archived"), small = true, muted = true)
                if (hooks.isEmpty())
                    Text(app.t("webhooks.empty"), Modifier.padding(vertical = 12.dp), muted = true)
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    hooks.forEach { hook ->
                        Column(
                            Modifier.fillMaxWidth()
                                .border(
                                    1.dp,
                                    MaterialTheme.colorScheme.outline,
                                    RoundedCornerShape(10.dp),
                                )
                                .padding(13.dp),
                            verticalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Text(hook.text("name"), weight = FontWeight.SemiBold)
                            WebhookAppSelect(app, channel.text("id"), hook) { revision++ }
                            WebhookCredential(
                                app,
                                app.t("webhooks.url"),
                                nativeAbsoluteUrl(
                                    endpoint(app.config.apiBaseUrl, hook.text("path"))
                                ),
                            )
                            Row(
                                Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                            ) {
                                Button(
                                    app.t("webhooks.rotate"),
                                    !busy,
                                    variant = "outline",
                                    size = "sm",
                                ) {
                                    confirmation = hook to "rotate"
                                }
                                Button(
                                    app.t("webhooks.delete"),
                                    !busy,
                                    variant = "destructive",
                                    size = "sm",
                                ) {
                                    confirmation = hook to "delete"
                                }
                            }
                            if (confirmation?.first?.text("id") == hook.text("id")) {
                                val action = confirmation!!.second
                                Column(
                                    Modifier.fillMaxWidth()
                                        .border(
                                            1.dp,
                                            MaterialTheme.colorScheme.outline,
                                            RoundedCornerShape(4.dp),
                                        )
                                        .padding(9.dp),
                                    verticalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    Text(
                                        app.t(
                                            if (action == "delete") "webhooks.deleteConfirm"
                                            else "webhooks.rotateConfirm"
                                        )
                                    )
                                    Row(
                                        Modifier.fillMaxWidth(),
                                        horizontalArrangement =
                                            Arrangement.spacedBy(8.dp, Alignment.End),
                                    ) {
                                        Button(
                                            app.t("common.cancelShort"),
                                            !busy,
                                            variant = "ghost",
                                            size = "sm",
                                        ) {
                                            confirmation = null
                                        }
                                        Button(
                                            app.t("webhooks.confirm"),
                                            !busy,
                                            variant = "destructive",
                                            size = "sm",
                                        ) {
                                            run {
                                                if (action == "delete") {
                                                    app.api.request(
                                                        "$path/${hook.text("id")}",
                                                        "DELETE",
                                                    )
                                                    hooks =
                                                        hooks.filter {
                                                            it.text("id") != hook.text("id")
                                                        }
                                                } else {
                                                    val response =
                                                        app.api
                                                            .request(
                                                                "$path/${hook.text("id")}/rotate-secret",
                                                                "POST",
                                                            )
                                                            .jsonObject
                                                    val changed =
                                                        response.nested("data").ifEmpty { response }
                                                    hooks =
                                                        hooks.map {
                                                            if (it.text("id") == hook.text("id"))
                                                                JsonObject(it + changed)
                                                            else it
                                                        }
                                                }
                                                confirmation = null
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun MessageAppContent(app: Session, message: Row) {
    val appId = message.text("app_origin_id").ifEmpty { message.text("app_id") }
    if (appId.isNotEmpty())
        Blocks(
            app,
            message.array("app_blocks"),
            appId,
            message = message.text("id"),
            channel = message.text("channel_id").ifEmpty { app.contextChannelId },
        )
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
            messageCards(unfurl["card"])?.singleOrNull()?.let { MessageCard(it) }
            if (unfurl.present("url") && app.contextChannelId != null)
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
                                        "channel_id" to app.contextChannelId,
                                    ),
                                )
                            }
                        }
                    }
                }
        }
}

@Composable
private fun AppsContainer(
    app: Session,
    embedded: Boolean,
    close: () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    if (embedded)
        Column(
            Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(16.dp),
            content = content,
        )
    else
        Panel(
            app.t("apps.title"),
            close,
            896.dp,
            description = app.t("apps.description"),
            content = content,
        )
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AppPermissions(app: Session, scopes: JsonArray) {
    SettingsSection(app.t("apps.permissions")) {
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            scopes.forEach { scope ->
                Text(
                    scope.jsonPrimitive.content,
                    Modifier.background(
                            MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .4f),
                            RoundedCornerShape(8.dp),
                        )
                        .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
                        .padding(horizontal = 9.dp, vertical = 5.dp),
                    small = true,
                    muted = true,
                )
            }
        }
    }
}

@Composable
private fun AppCredential(app: Session, value: String) {
    SettingsSection("") {
        Text(app.t("apps.credential"), small = true, muted = true)
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Field(
                value,
                app.t("apps.credential"),
                Modifier.weight(1f),
                secret = true,
                readOnly = true,
                showLabel = false,
                inputTextStyle = TextStyle(fontFamily = FontFamily.Monospace),
            ) {}
            Button(app.t("apps.copy"), variant = "outline") {
                app.run {
                    try {
                        app.platform.invoke("copy", value)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        app.toast = app.t("apps.actionError")
                    }
                }
            }
        }
    }
}

/** CSS grid stretches this column to the detail's natural height, not the viewport height. */
@Composable
private fun AppSidebarColumn(
    modifier: Modifier = Modifier,
    minimumHeight: Dp = 0.dp,
    content: @Composable () -> Unit,
) {
    Layout(content = content, modifier = modifier) { measurables, constraints ->
        check(measurables.size == 2)
        val unbounded = constraints.copy(minHeight = 0, maxHeight = Constraints.Infinity)
        val controls = measurables[0].measure(unbounded)
        val footer = measurables[1].measure(unbounded)
        val height =
            constraints.constrainHeight(
                maxOf(minimumHeight.roundToPx(), controls.height + footer.height)
            )
        val width = constraints.constrainWidth(maxOf(controls.width, footer.width))
        layout(width, height) {
            controls.placeRelative(0, 0)
            footer.placeRelative(0, maxOf(controls.height, height - footer.height))
        }
    }
}
