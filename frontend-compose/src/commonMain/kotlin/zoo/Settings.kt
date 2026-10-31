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
    val tab = app.query["tab"].takeIf { it in listOf("apps", "members") } ?: "general"
    val tabs =
        listOf(
            "general" to "settings.general",
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
    Column(Modifier.fillMaxSize()) {
        PaneHeader {
            IconButton("arrow-left", app.t("channels.back"), size = 44.dp) { app.navigate("/") }
            Text(
                app.t("settings.zooTitle"),
                Modifier.semantics { heading() },
                weight = FontWeight.SemiBold,
                size = 18.sp,
            )
        }
        HorizontalDivider()
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(24.dp),
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
                    .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(8.dp))
                    .padding(4.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                tabs.forEachIndexed { index, (value, label) ->
                    Button(
                        app.t(label),
                        role = Role.Tab,
                        selected = tab == value,
                        modifier =
                            Modifier.focusRequester(tabFocus[index])
                                .background(
                                    if (tab == value) MaterialTheme.colorScheme.background
                                    else androidx.compose.ui.graphics.Color.Transparent,
                                    RoundedCornerShape(6.dp),
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
                else -> GeneralSettings(app)
            }
        }
    }
}

@Composable
private fun GeneralSettings(app: Session) {
    Text(app.t("theme.title"), weight = FontWeight.Medium)
    SelectField(
        app.t("theme.title"),
        app.t("theme.${app.env.theme}"),
        listOf("light", "dark", "system").map { app.t("theme.$it") },
        allowEmpty = false,
    ) { label ->
        listOf("light", "dark", "system")
            .firstOrNull { app.t("theme.$it") == label }
            ?.let { app.platform.action("theme:$it") }
    }
    HorizontalDivider()
    Text(app.t("language.title"), weight = FontWeight.Medium)
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
    HorizontalDivider()
    var attempt by remember { mutableStateOf(0) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf(false) }
    LaunchedEffect(attempt, app.env.online) {
        loading = true
        try {
            app.searchEngine =
                app.api
                    .request("/api/users/me/preferences")
                    .jsonObject
                    .text("search_engine")
                    .takeIf { it in searchEngines }
                    ?: throw IllegalArgumentException("Invalid engine")
            error = false
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            error = true
        } finally {
            loading = false
        }
    }
    Text(app.t("search.engine"), weight = FontWeight.Medium)
    if (loading) Text(app.t("common.loading"), muted = true)
    else if (error) {
        Text(app.t("search.loadError"))
        Button(app.t("search.retry")) { attempt++ }
    } else
        SelectField(
            app.t("search.engine"),
            searchEngines[app.searchEngine].orEmpty(),
            searchEngines.values.toList(),
            enabled = "preferences" !in app.pending,
            allowEmpty = false,
        ) { label ->
            val next = searchEngines.entries.first { it.value == label }.key
            app.run("preferences") {
                try {
                    app.api.request(
                        "/api/users/me/preferences",
                        "PATCH",
                        obj("search_engine" to next),
                    )
                    app.searchEngine = next
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    app.toast = app.t("search.saveError")
                }
            }
        }
    HorizontalDivider()
    Text(app.t("pwa.title"), weight = FontWeight.Medium)
    if (app.capabilities.flag("installAvailable"))
        Button(app.t("pwa.install")) { app.run { app.platform.invoke("install") } }
    Button(
        app.t(if (app.capabilities.flag("pushEnabled")) "pwa.disable" else "pwa.enable"),
        app.capabilities.flag("pushSupported") && app.capabilities.flag("pushConfigured"),
    ) {
        app.run("push") {
            app.platform.invoke("push")
            app.capabilities =
                json.parseToJsonElement(app.platform.invoke("capabilities")).jsonObject
        }
    }
    if (!app.capabilities.flag("pushSupported")) Text(app.t("pwa.unsupported"), muted = true)
    else if (!app.capabilities.flag("pushConfigured")) Text(app.t("pwa.unconfigured"), muted = true)
}

fun canChangeAdmin(userId: String, admins: Set<String>, protected: Set<String>): Boolean =
    userId !in protected && (userId !in admins || admins.size > 1)

@Composable
fun WorkspaceMembers(app: Session, users: List<Row>) {
    var query by remember { mutableStateOf("") }
    var target by remember { mutableStateOf<Row?>(null) }
    var revision by remember { mutableStateOf(0) }
    Text(app.t("settings.membersDescription"), muted = true)
    Field(query, app.t("members.search")) { query = it }
    ApiContent("/api/users/admins", revision) { data, _ ->
        val admins =
            data.jsonObject["user_ids"]
                ?.jsonArray
                ?.map { it.jsonPrimitive.content }
                ?.toSet()
                .orEmpty()
        val protected =
            data.jsonObject["protected_user_ids"]
                ?.jsonArray
                ?.map { it.jsonPrimitive.content }
                ?.toSet()
                .orEmpty()
        val filtered =
            users
                .filter {
                    !it.flag("is_bot") &&
                        (author(it) + " " + it.text("user_name")).contains(query.trim(), true)
                }
                .sortedBy { author(it).lowercase() }
        filtered.forEach { user ->
            val id = user.text("id")
            Row(
                Modifier.fillMaxWidth().padding(vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Avatar(user)
                Column(Modifier.weight(1f)) {
                    Text(author(user), weight = FontWeight.Medium)
                    Text(
                        "@${user.text("user_name")} · ${app.t(if (id in admins) "settings.admin" else "settings.member")}",
                        small = true,
                        muted = true,
                    )
                }
                if (app.config.isAdmin)
                    Button(
                        app.t(if (id in admins) "settings.removeAdmin" else "settings.makeAdmin"),
                        app.env.online && canChangeAdmin(id, admins, protected),
                    ) {
                        target = user
                    }
            }
            HorizontalDivider()
        }
        if (filtered.isEmpty()) Text(app.t("members.usersEmpty"), muted = true)
        target?.let { user ->
            val id = user.text("id")
            val isAdmin = id in admins
            val label = app.t(if (isAdmin) "settings.removeAdmin" else "settings.makeAdmin")
            Confirm(
                label,
                app.t(
                    "settings.roleConfirm",
                    "name" to author(user),
                    "role" to app.t(if (isAdmin) "settings.member" else "settings.admin"),
                ),
                label,
                "admin:$id" in app.pending,
                { target = null },
            ) {
                app.run("admin:$id") {
                    try {
                        app.api.request(
                            "/api/users/$id/admin",
                            "PATCH",
                            buildJsonObject { put("is_admin", !isAdmin) },
                        )
                        target = null
                        revision++
                        if (id == app.config.userId) app.platform.invoke("refreshUser")
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        app.toast =
                            app.t(
                                if ((e as? ApiException)?.status == 409) "settings.lastAdmin"
                                else "common.writeError"
                            )
                    }
                }
            }
        }
    }
}
