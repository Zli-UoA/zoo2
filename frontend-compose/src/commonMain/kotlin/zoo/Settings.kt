package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.serialization.json.*

val searchEngines =
    linkedMapOf(
        "google" to "Google",
        "duckduckgo" to "DuckDuckGo",
        "yandex" to "Yandex",
        "baidu" to "Baidu",
    )

fun searchUrl(engine: String, query: String): String {
    val (url, key) =
        when (engine) {
            "duckduckgo" -> "https://duckduckgo.com/" to "q"
            "yandex" -> "https://yandex.com/search/" to "text"
            "baidu" -> "https://www.baidu.com/s" to "wd"
            else -> "https://www.google.com/search" to "q"
        }
    return io.ktor.http.URLBuilder(url).apply { parameters[key] = query }.buildString()
}

@Composable
fun ZooSettings(app: Session, users: List<Row>) {
    val tab =
        app.query["tab"].takeIf { it in listOf("apps", "members", "activitypub") } ?: "general"
    val tabs =
        listOf(
            "general" to "settings.general",
            "activitypub" to "ActivityPub",
            "apps" to "apps.title",
            "members" to "members.title",
        )
    val tabFocus = remember { List(tabs.size) { FocusRequester() } }
    fun selectTab(value: String) {
        val builder =
            io.ktor.http.URLBuilder("https://zoo.invalid${app.env.path}").apply {
                parameters["tab"] = value
            }
        app.platform.replace(builder.buildString().removePrefix("https://zoo.invalid"))
    }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val contentPadding = if (maxWidth < 640.dp) 16.dp else 24.dp
        Column(Modifier.fillMaxSize()) {
            PaneHeader {
                Button(
                    app.t("channels.back"),
                    variant = "ghost",
                    size = "icon-sm",
                    icon = "arrow-left",
                    modifier = Modifier.size(44.dp),
                ) {
                    app.platform.action("back")
                }
                Spacer(Modifier.width(12.dp))
                Text(
                    app.t("settings.zooTitle"),
                    Modifier.semantics { heading() },
                    weight = FontWeight.SemiBold,
                    size = 18.sp,
                    lineHeight = 28.sp,
                )
            }
            HorizontalDivider()
            Column(
                Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(contentPadding),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Row(
                    Modifier.fillMaxWidth()
                        .then(webControlModifier("tablist"))
                        .onPreviewKeyEvent { event ->
                            if (
                                event.type != KeyEventType.KeyDown ||
                                    event.isCtrlPressed ||
                                    event.isMetaPressed ||
                                    event.isAltPressed
                            )
                                false
                            else {
                                val current = tabs.indexOfFirst { it.first == tab }
                                val next =
                                    when (event.key) {
                                        Key.DirectionRight -> (current + 1) % tabs.size
                                        Key.DirectionLeft -> (current + tabs.size - 1) % tabs.size
                                        Key.MoveHome -> 0
                                        Key.MoveEnd -> tabs.lastIndex
                                        else -> -1
                                    }
                                if (next < 0) false
                                else {
                                    selectTab(tabs[next].first)
                                    tabFocus[next].requestFocus()
                                    true
                                }
                            }
                        }
                        .background(
                            MaterialTheme.colorScheme.surfaceVariant,
                            RoundedCornerShape(10.dp),
                        )
                        .padding(4.dp),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    tabs.forEachIndexed { index, (value, label) ->
                        Button(
                            app.t(label),
                            role = Role.Tab,
                            variant = "tab",
                            selected = tab == value,
                            modifier =
                                Modifier.focusRequester(tabFocus[index])
                                    .background(
                                        if (tab == value) MaterialTheme.colorScheme.background
                                        else androidx.compose.ui.graphics.Color.Transparent,
                                        RoundedCornerShape(8.dp),
                                    )
                                    .semantics { selected = tab == value },
                        ) {
                            selectTab(value)
                        }
                    }
                }
                when (tab) {
                    "apps" -> Apps(app, embedded = true)
                    "members" -> WorkspaceMembers(app, users)
                    "activitypub" -> ActivityPubSettingsTab(app)
                    else -> GeneralSettings(app)
                }
            }
        }
    }
}

@Composable
private fun GeneralSettings(app: Session) {
    Column(
        Modifier.fillMaxWidth()
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(10.dp))
            .padding(1.dp)
    ) {
        SettingRow(app.t("theme.title"), icon = "palette") { ThemeToggle(app) }
        HorizontalDivider()
        SettingRow(app.t("language.title"), icon = "languages") { LanguageSelect(app) }
        HorizontalDivider()
        SettingRow(app.t("search.engine"), icon = "search") {
            Column(
                horizontalAlignment = Alignment.End,
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                SearchEngineSelect(app)
            }
        }
        HorizontalDivider()
        SettingRow(app.t("pwa.title"), icon = "bell") { PwaSelect(app) }
    }
}

@Composable
fun LanguageSelect(app: Session) {
    val languages =
        linkedMapOf(
            "ja" to "日本語",
            "en" to "English",
            "zh-Hant" to "繁體中文",
            "zh-Hans" to "简体中文",
            "yue" to "廣東話",
        )
    SelectField(
        app.t("language.title"),
        languages[app.env.locale].orEmpty(),
        languages.values.toList(),
        allowEmpty = false,
    ) { label ->
        app.platform.action("locale:${languages.entries.first { it.value == label }.key}")
    }
}

@Composable
fun SearchEngineSelect(app: Session) {
    val preferences = app.searchPreferences
    LaunchedEffect(preferences) { preferences.ensureLoaded() }
    val loading = preferences.loading
    val saving = preferences.saving
    val error = preferences.error
    Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SelectField(
            app.t("search.engine"),
            searchEngines[app.searchEngine].orEmpty(),
            searchEngines.values.toList(),
            enabled = !loading && !saving && error != "load",
            busy = loading || saving,
            allowEmpty = false,
        ) { label ->
            preferences.save(searchEngines.entries.first { it.value == label }.key)
        }
        if (error.isNotEmpty()) {
            val message = app.t(if (error == "load") "search.loadError" else "search.saveError")
            val fonts = LocalFonts.current
            val measure = androidx.compose.ui.text.rememberTextMeasurer()
            val density = androidx.compose.ui.platform.LocalDensity.current
            val style = MaterialTheme.typography.bodyMedium
            val textLayout = measure.measure(fontText(message, fonts), style)
            val textWidth = textLayout.getLineRight(0) - textLayout.getLineLeft(0)
            val retryWidth =
                if (error == "load") {
                    val retryLayout =
                        measure.measure(
                            fontText(app.t("search.retry"), fonts, FontWeight.Medium),
                            style.copy(fontWeight = FontWeight.Medium),
                        )
                    retryLayout.getLineRight(0) - retryLayout.getLineLeft(0) +
                        with(density) { 22.dp.toPx() }
                } else 0f
            val preferredWidth =
                with(density) {
                    kotlin.math.ceil(textWidth + retryWidth).toDp().coerceAtMost(256.dp)
                }
            CssFlowRow(
                Modifier.width(preferredWidth).then(webControlModifier("alert")).semantics {
                    liveRegion = LiveRegionMode.Polite
                },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    message,
                    Modifier.cssPreferredWidth(textWidth),
                    color = MaterialTheme.colorScheme.error,
                )
                if (error == "load")
                    Button(app.t("search.retry"), variant = "link") { preferences.retry() }
            }
        }
    }
}

@Composable
fun PwaSelect(app: Session) {
    val enabled = app.capabilities.flag("pushEnabled")
    val supported = app.capabilities.flag("pushSupported")
    val configured = app.capabilities.flag("pushConfigured")
    val blocked = app.capabilities.flag("pushBlocked")
    val unavailable = !supported || !enabled && (!configured || blocked)
    Column(
        Modifier.width(160.dp),
        horizontalAlignment = Alignment.End,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(Modifier.width(160.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            SelectField(
                app.t("pwa.title"),
                app.t(if (unavailable) "pwa.unavailable" else if (enabled) "pwa.on" else "pwa.off"),
                (if (unavailable) listOf(app.t("pwa.unavailable")) else emptyList()) +
                    listOf(app.t("pwa.on"), app.t("pwa.off")),
                enabled =
                    !unavailable && !app.capabilities.flag("pushBusy") && "push" !in app.pending,
                busy = app.capabilities.flag("pushBusy") || "push" in app.pending,
                allowEmpty = false,
            ) { value ->
                if ((value == app.t("pwa.on")) != enabled)
                    app.run("push") {
                        app.platform.invoke("push")
                        app.capabilities =
                            json
                                .parseToJsonElement(app.platform.invoke("capabilities"))
                                .jsonObject
                                .ifEmpty { app.capabilities }
                    }
            }
            if (unavailable)
                Text(
                    app.capabilities.text("pushLabel").ifEmpty {
                        app.t(
                            if (!supported) "pwa.unsupported"
                            else if (!configured) "pwa.unconfigured" else "pwa.blocked"
                        )
                    },
                    muted = true,
                    small = true,
                )
        }
        if (app.capabilities.flag("installAvailable"))
            Button(app.t("pwa.install"), variant = "outline", size = "sm", icon = "download") {
                app.run {
                    app.platform.invoke("install")
                    app.capabilities =
                        JsonObject(app.capabilities + ("installAvailable" to JsonPrimitive(false)))
                }
            }
    }
}

fun canChangeAdmin(userId: String, admins: Set<String>, protected: Set<String>): Boolean =
    userId !in protected && (userId !in admins || admins.size > 1)

@Composable
fun WorkspaceMembers(app: Session, users: List<Row>) {
    var query by remember { mutableStateOf("") }
    var target by remember { mutableStateOf<Row?>(null) }
    var admins by remember { mutableStateOf<Set<String>>(emptySet()) }
    var protected by remember { mutableStateOf<Set<String>>(emptySet()) }
    var loading by remember { mutableStateOf(true) }
    var pending by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var admin by remember { mutableStateOf(app.config.isAdmin) }
    var revision by remember { mutableStateOf(0) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(app) {
        try {
            admin = app.api.request("/api/users/me").jsonObject.flag("is_admin")
        } catch (cause: CancellationException) {
            throw cause
        } catch (_: Exception) {}
    }
    LaunchedEffect(app, revision) {
        loading = true
        try {
            val data = app.api.request("/api/users/admins").jsonObject
            admins = data.array("user_ids").map { it.jsonPrimitive.content }.toSet()
            protected = data.array("protected_user_ids").map { it.jsonPrimitive.content }.toSet()
            error = ""
        } catch (cause: CancellationException) {
            throw cause
        } catch (_: Exception) {
            error = app.t("settings.membersLoadError")
        } finally {
            loading = false
        }
    }
    suspend fun changeRole(user: Row, wasAdmin: Boolean): Boolean {
        if (pending) return false
        pending = true
        error = ""
        return try {
            val id = user.text("id")
            app.api.request("/api/users/$id/admin", "PATCH", obj("is_admin" to !wasAdmin))
            admins = if (wasAdmin) admins - id else admins + id
            if (id == app.config.userId) {
                app.platform.invoke("refreshUser")
                admin = !wasAdmin
            }
            target = null
            true
        } catch (cause: CancellationException) {
            throw cause
        } catch (cause: Exception) {
            error =
                app.t(
                    if ((cause as? ApiException)?.status == 409) "settings.lastAdmin"
                    else "common.writeError"
                )
            false
        } finally {
            pending = false
        }
    }
    val filtered =
        users
            .filter {
                !it.flag("is_bot") &&
                    (author(it) + " " + it.text("user_name")).contains(query.trim(), true)
            }
            .sortedWith { left, right ->
                compareLocaleText(author(left), author(right), app.env.locale)
            }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(app.t("settings.membersDescription"), muted = true)
        Field(
            query,
            app.t("members.search"),
            showLabel = false,
            placeholder = app.t("members.search"),
            inputRole = "searchbox",
        ) {
            query = it
        }
        if (loading)
            Text(
                app.t("common.loading"),
                Modifier.then(webControlModifier("status")).semantics {
                    liveRegion = LiveRegionMode.Polite
                },
                size = 16.sp,
                lineHeight = 24.sp,
            )
        if (error.isNotEmpty() && target == null)
            Row(
                Modifier.then(webControlModifier("alert")).semantics {
                    liveRegion = LiveRegionMode.Polite
                },
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(error, color = MaterialTheme.colorScheme.error)
                Button(
                    app.t("offline.retry"),
                    variant = "outline",
                    textStyle =
                        MaterialTheme.typography.bodyMedium.copy(
                            color = MaterialTheme.colorScheme.error
                        ),
                ) {
                    revision++
                }
            }
        Column(
            Modifier.fillMaxWidth()
                .then(webControlModifier("list"))
                .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(10.dp))
                .padding(1.dp)
        ) {
            filtered.forEachIndexed { index, user ->
                val id = user.text("id")
                Row(
                    Modifier.fillMaxWidth().then(webControlModifier("listitem")).padding(16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Avatar(user, size = 24.dp)
                    Column(Modifier.weight(1f)) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Text(
                                author(user),
                                Modifier.weight(1f, fill = false),
                                weight = FontWeight.Medium,
                                maxLines = 1,
                            )
                            if (id == app.config.userId)
                                Text(app.t("members.you"), muted = true, small = true)
                        }
                        Text("@${user.text("user_name")}", muted = true, small = true, maxLines = 1)
                    }
                    if (id in admins)
                        Box(
                            Modifier.background(
                                    MaterialTheme.colorScheme.secondaryContainer,
                                    RoundedCornerShape(8.dp),
                                )
                                .padding(horizontal = 8.dp, vertical = 4.dp)
                        ) {
                            Text(
                                app.t("settings.admin"),
                                small = true,
                                color = MaterialTheme.colorScheme.onSecondaryContainer,
                            )
                        }
                    if (admin)
                        Button(
                            app.t(
                                if (id in admins) "settings.removeAdmin" else "settings.makeAdmin"
                            ),
                            !loading &&
                                !pending &&
                                error.isEmpty() &&
                                canChangeAdmin(id, admins, protected),
                            variant = "outline",
                            size = "sm",
                        ) {
                            error = ""
                            target = user
                        }
                }
                if (index < filtered.lastIndex) HorizontalDivider()
            }
        }
        if (!loading && filtered.isEmpty()) Text(app.t("members.usersEmpty"), muted = true)
    }
    target?.let { user ->
        val wasAdmin = user.text("id") in admins
        val label = app.t(if (wasAdmin) "settings.removeAdmin" else "settings.makeAdmin")
        ConfirmationPanel(
            label,
            app.t(
                "settings.roleConfirm",
                "name" to author(user),
                "role" to app.t(if (wasAdmin) "settings.member" else "settings.admin"),
            ),
            label,
            pending,
            error,
            {
                if (!pending) {
                    target = null
                    error = ""
                }
            },
            { scope.launch { changeRole(user, wasAdmin) } },
        )
    }
}

@Composable
private fun ActivityPubSettingsTab(app: Session) {
    val settingsScope = rememberCoroutineScope()
    val channelRules =
        remember(app) {
            ChannelTimelineSettingsController(settingsScope) { method, body ->
                app.api.request("/api/home/settings", method, body).jsonObject
            }
        }
    LaunchedEffect(channelRules) { channelRules.load() }
    DisposableEffect(channelRules) { onDispose { channelRules.close() } }
    val profile =
        remember(app) {
            ActivityPubSettingsController(settingsScope) { method, body ->
                app.api.request("/api/activitypub/profile", method, body).jsonObject
            }
        }
    LaunchedEffect(profile) { profile.load() }
    DisposableEffect(profile) { onDispose { profile.close() } }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ActivityPubSettings(app, profile)
        ChannelTimelineSettings(app, channelRules)
    }
}
