@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import androidx.compose.runtime.*
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.window.ComposeViewport
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlin.JsFun
import kotlin.js.*
import kotlinx.browser.document
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

@JsFun(
    "(boot) => { globalThis.__zooComposeCatalogBoot = boot; globalThis.dispatchEvent(new Event('zoo-compose-catalog-module-ready')); }"
)
private external fun catalogBoot(boot: () -> Unit)

@JsFun(
    "(value, locale) => { const date = new Date(value.replace(' ', 'T').replace(/(?<!Z)(?<![+-][0-9]{2}:[0-9]{2})$/, 'Z')); return Number.isNaN(date.getTime()) ? value : new Intl.DateTimeFormat(locale, {month:'short',day:'numeric',hour:'2-digit',minute:'2-digit',hourCycle:'h23'}).format(date); }"
)
private external fun formatNotification(value: String, locale: String): String

@JsFun("(payload) => globalThis.zooCatalog.callback(payload)")
private external fun storyCallback(payload: String): Promise<JsString>

@JsFun("(payload) => navigator.clipboard.writeText(payload).then(() => '')")
private external fun copyCatalogText(payload: String): Promise<JsString>

@JsFun(
    """async url => {const response=await fetch(url,{credentials:'omit',referrerPolicy:'no-referrer'});if(!response.ok)throw new Error('Image HTTP '+response.status);const blob=await response.blob();const source=URL.createObjectURL(blob);try{const img=new Image();img.src=source;await img.decode();const canvas=document.createElement('canvas');canvas.width=img.naturalWidth;canvas.height=img.naturalHeight;canvas.getContext('2d').drawImage(img,0,0);return canvas.toDataURL('image/png').split(',')[1];}finally{URL.revokeObjectURL(source);} }"""
)
private external fun catalogImage(url: String): Promise<JsString>

@JsFun("() => JSON.stringify(globalThis.zooCatalog.options)")
private external fun catalogOptions(): String

@JsFun("(update, close, escape) => globalThis.zooCatalog.ready(update, close, escape)")
private external fun catalogReady(
    update: (String) -> Unit,
    close: () -> Unit,
    escape: () -> Boolean,
)

@JsFun("(message) => globalThis.zooCatalog.painted(message)")
private external fun catalogPainted(message: String)

@JsFun("(width,height) => globalThis.zooCatalog.measured(width,height)")
private external fun catalogMeasured(width: Double, height: Double)

@JsFun("(action) => globalThis.zooCatalog.preference(action)")
private external fun catalogPreference(action: String)

@JsFun("(top) => globalThis.zooCatalog.scroll(top)") private external fun catalogScroll(top: Double)

@JsFun("(open) => globalThis.zooCatalog.overlay(open)")
private external fun catalogOverlay(open: Boolean)

@JsFun("() => globalThis.zooCatalog") private external fun catalogFileBridge(): JsAny

@JsFun(
    "(url,newTab) => { dispatchEvent(new CustomEvent('zoo:navigation-intent',{detail:{url,newTab}})); if(newTab) window.open(url,'_blank','noopener,noreferrer'); }"
)
private external fun catalogNavigationIntent(url: String, newTab: Boolean)

@JsFun("() => location.reload()") private external fun reloadCatalogPage()

@JsFun("() => matchMedia('(prefers-color-scheme: dark)').matches")
private external fun systemDark(): Boolean

// Storybook's same-origin MSW worker supplies the same fixtures to both renderers.
@JsFun(
    "(path, method, body) => fetch(path, {method, credentials: 'omit', headers: body ? {'Content-Type':'application/json'} : {}, ...(body ? {body} : {})}).then(async r => JSON.stringify({status:r.status, body:await r.text()}))"
)
private external fun fixtureApi(path: String, method: String, body: String): Promise<JsString>

@JsFun(
    "url => fetch(new URL(url).hostname === 'catalog.invalid' ? new URL(url).pathname + new URL(url).search : url, {credentials:'omit'}).then(async r => {if(!r.ok) throw Error('Fixture fetch failed'); return JSON.stringify({body:await r.text(), contentType:r.headers.get('Content-Type') || ''})})"
)
private external fun fixturePublic(url: String): Promise<JsString>

internal fun registerCatalog() = catalogBoot(::startCatalog)

@OptIn(ExperimentalComposeUiApi::class)
private fun startCatalog() {
    org.jetbrains.compose.resources.configureWebResources { resourcePathMapping { "/compose/$it" } }
    var options by mutableStateOf(json.parseToJsonElement(catalogOptions()).jsonObject)
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val storage = mutableMapOf<String, String>()
    var sequence = 0
    lateinit var app: Session
    lateinit var refresh: () -> Unit
    val files = BrowserFileBridge(catalogFileBridge())
    val navigationHistory = mutableListOf<Pair<String, Row>>()
    var navigationIndex = 0
    var hostPath = options.text("path")
    fun catalogNavigate(path: String, state: Row, replace: Boolean) {
        if (navigationHistory.isEmpty()) navigationHistory.add(app.env.path to app.env.historyState)
        if (!replace) {
            while (navigationHistory.size > navigationIndex + 1) navigationHistory.removeAt(
                navigationHistory.lastIndex
            )
            navigationIndex++
        }
        val entry = path to JsonObject(state + ("__TSR_index" to JsonPrimitive(navigationIndex)))
        if (replace) navigationHistory[navigationIndex] = entry else navigationHistory.add(entry)
        catalogNavigationIntent(path, false)
        options =
            JsonObject(
                options + mapOf("path" to JsonPrimitive(path), "historyState" to entry.second)
            )
        refresh()
    }
    val platform =
        object : Platform {
            override fun fileTarget(id: String, bounds: String, receive: (String) -> Boolean) =
                files.target(id, bounds, receive)

            override fun forgetFileTarget(id: String) = files.forget(id)

            override fun controlSemantics(
                id: String,
                role: String,
                enabled: Boolean,
                selected: Boolean?,
                expanded: Boolean?,
                attributes: Map<String, String>,
            ) = webControl(id, role, enabled, selected, expanded, attributes)

            override fun outsideClick(id: String, bounds: String, close: () -> Unit) =
                observeOutsideClick(id, bounds, close)

            override fun forgetOutsideClick(id: String) = removeOutsideClick(id)

            override fun controlFocus(id: String, focus: () -> Unit) = webControlFocus(id, focus)

            override fun forgetControlSemantics(id: String) = forgetWebControl(id)

            override suspend fun image(url: String): ByteArray =
                kotlin.io.encoding.Base64.decode(
                    catalogImage(url).awaitBrowser<JsString>().toString()
                )

            override suspend fun rasterizeSvg(svg: String, width: Int, height: Int): ByteArray =
                browserRasterizeSvg(svg, width, height)

            override suspend fun headers() = emptyMap<String, String>()

            override suspend fun readCache(key: String): String? {
                val fixture = options.nested("fixture")
                val fields =
                    mapOf(
                        "channels" to "channels",
                        "users" to "users",
                        "channel_users" to "channelUsers",
                        "messages" to "messages",
                        "message_reactions" to "reactions",
                        "attachments" to "attachments",
                        "notifications" to "notifications",
                    )
                val table = fields.keys.firstOrNull { key.contains(":$it:") } ?: return "[]"
                if (fixture.array("loadingTables").any { it.jsonPrimitive.content == table })
                    return null
                val rows =
                    (fixture[fields.getValue(table)] as? JsonArray)?.map { it.jsonObject }
                        ?: when (table) {
                            "users" -> CatalogSamples.users
                            "channels" -> listOf(CatalogSamples.channel)
                            else -> emptyList()
                        }
                val channelId = Regex("channel_id=([^,\\]]+)").find(key)?.groupValues?.get(1)
                val selected =
                    if (
                        channelId == null ||
                            table !in listOf("messages", "attachments", "message_reactions")
                    )
                        rows
                    else {
                        val ids =
                            fixture
                                .array("messages")
                                .map { it.jsonObject }
                                .filter { it.text("channel_id") == channelId }
                                .map { it.text("id") }
                                .toSet()
                        rows.filter {
                            if (table == "messages") it.text("channel_id") == channelId
                            else it.text("message_id") in ids || it.text("channel_id") == channelId
                        }
                    }
                return JsonArray(selected).toString()
            }

            override suspend fun writeCache(key: String, value: String) {}

            override fun uuid() = "catalog-${++sequence}"

            override fun now() = webCurrentTime()

            override fun navigate(path: String) =
                catalogNavigate(path, JsonObject(emptyMap()), false)

            override fun navigate(path: String, state: Row) = catalogNavigate(path, state, false)

            override fun replace(path: String) = catalogNavigate(path, JsonObject(emptyMap()), true)

            override fun action(name: String) {
                when {
                    name == "react" || name == "compose" -> {
                        catalogPreference("frontend:$name")
                        return
                    }
                    name.startsWith("open:") -> {
                        catalogNavigationIntent(name.substringAfter(':'), true)
                        return
                    }
                    name == "reload" -> {
                        reloadCatalogPage()
                        return
                    }
                    name == "back" -> {
                        if (navigationIndex > 0) {
                            navigationIndex--
                            val entry = navigationHistory[navigationIndex]
                            catalogNavigationIntent(entry.first, false)
                            options =
                                JsonObject(
                                    options +
                                        mapOf(
                                            "path" to JsonPrimitive(entry.first),
                                            "historyState" to entry.second,
                                        )
                                )
                            refresh()
                        } else replace("/")
                        return
                    }
                    name.startsWith("fileFocus:") || name.startsWith("fileRelease:") -> {
                        files.action(name)
                        return
                    }
                    name.startsWith("catalog-overlay:") -> {
                        catalogOverlay(name.substringAfter(':') == "true")
                        return
                    }
                    name.startsWith("catalog-scroll:") -> {
                        name
                            .substringAfter(':')
                            .toDoubleOrNull()
                            ?.takeIf { it.isFinite() && it >= 0 }
                            ?.let(::catalogScroll)
                        return
                    }
                    name.startsWith("catalog-size:") -> {
                        catalogMeasured(
                            name.split(':')[1].toDouble(),
                            name.split(':')[2].toDouble(),
                        )
                        return
                    }
                    name.startsWith("theme:") -> {
                        val theme = name.substringAfter(':')
                        options =
                            JsonObject(
                                options +
                                    mapOf(
                                        "theme" to JsonPrimitive(theme),
                                        "dark" to
                                            JsonPrimitive(
                                                theme == "dark" || theme == "system" && systemDark()
                                            ),
                                    )
                            )
                    }
                    name.startsWith("locale:") ->
                        options =
                            JsonObject(
                                options + ("locale" to JsonPrimitive(name.substringAfter(':')))
                            )
                    else -> return
                }
                catalogPreference(name)
                refresh()
            }

            override fun time(timestamp: String) =
                webFormatDate(timestamp, options.text("locale"), "time", 0)

            override fun notificationTime(timestamp: String) =
                formatNotification(timestamp, options.text("locale"))

            override fun day(timestamp: String) =
                webFormatDate(timestamp, options.text("locale"), "day", 0)

            override fun previousDay(timestamp: String) =
                webFormatDate(timestamp, options.text("locale"), "day", -1)

            override fun fullTimestamp(timestamp: String) =
                webFormatDate(timestamp, options.text("locale"), "full", 0)

            override fun translate(key: String, args: Map<String, String>): String {
                val labels =
                    options.nested("locales").nested(options.text("locale")).ifEmpty {
                        options.nested("labels")
                    }
                val suffix =
                    if (options.text("locale") == "en" && args["count"] == "1") "_one" else "_other"
                var value =
                    (if ("count" in args) labels.text(key + suffix) else "")
                        .ifEmpty { labels.text(key) }
                        .ifEmpty { key }
                args.forEach { (name, text) -> value = value.replace("{{$name}}", text) }
                return value
            }

            override suspend fun invoke(name: String, payload: String): String =
                when (name) {
                    "pickFiles",
                    "fileRead",
                    "filePut",
                    "attachmentDownload" -> files.invoke(name, payload)
                    "push" ->
                        if (options.nested("fixture").flag("pushToggleCallback"))
                            storyCallback(
                                    obj("name" to "pushToggle", "args" to JsonArray(emptyList()))
                                        .toString()
                                )
                                .awaitBrowser<JsString>()
                                .toString()
                        else "{}"
                    "copy" -> copyCatalogText(payload).awaitBrowser<JsString>().toString()
                    "storyCallback" -> storyCallback(payload).awaitBrowser<JsString>().toString()
                    "publicGet" ->
                        fixturePublic(json.parseToJsonElement(payload).jsonObject.text("url"))
                            .awaitBrowser<JsString>()
                            .toString()
                    "storageRead" -> storage[payload].orEmpty()
                    "storageWrite" -> {
                        val row = json.parseToJsonElement(payload).jsonObject
                        storage[row.text("key")] = row.text("value")
                        ""
                    }
                    else -> "{}"
                }
        }
    // All REST requests stay in memory. The catalog never starts Electric or Session.start().
    val http =
        HttpClient(
            MockEngine { request ->
                val path = request.url.encodedPath
                if (
                    path == "/api/messages" &&
                        request.method == HttpMethod.Post &&
                        options.nested("fixture").flag("postFailure")
                )
                    return@MockEngine respond("{}", HttpStatusCode.InternalServerError)
                if (!path.startsWith("/api/") || path.startsWith("/api/shapes/"))
                    awaitCancellation()
                if (options.nested("fixture").flag("sharedApi")) {
                    val response =
                        json
                            .parseToJsonElement(
                                fixtureApi(
                                        path +
                                            (if (request.url.encodedQuery.isEmpty()) ""
                                            else "?${request.url.encodedQuery}"),
                                        request.method.value,
                                        request.body.toByteArray().decodeToString(),
                                    )
                                    .awaitBrowser<JsString>()
                                    .toString()
                            )
                            .jsonObject
                    return@MockEngine respond(
                        response.text("body"),
                        HttpStatusCode.fromValue(response.text("status").toInt()),
                        headersOf(HttpHeaders.ContentType, "application/json"),
                    )
                }
                val body =
                    when {
                        path.startsWith("/api/attachments/") ->
                            obj(
                                    "download_url" to
                                        endpoint(options.text("origin"), "/icons/icon-192.png")
                                )
                                .toString()
                        path.endsWith("/preferences") -> obj("search_engine" to "google").toString()
                        path.endsWith("/admins") ->
                            buildJsonObject {
                                    put("user_ids", JsonArray(listOf(JsonPrimitive("alice"))))
                                    put(
                                        "protected_user_ids",
                                        JsonArray(listOf(JsonPrimitive("alice"))),
                                    )
                                }
                                .toString()
                        path.endsWith("/commands") -> "[]"
                        path.endsWith("/context") -> {
                            if (options.text("sample") == "notifications-loading") delay(60000)
                            buildJsonObject {
                                    put("message", obj("content" to CatalogSamples.message))
                                    put("reactions", JsonArray(emptyList()))
                                }
                                .toString()
                        }
                        path.endsWith("/bubble") -> {
                            if (request.method == HttpMethod.Get) "[]"
                            else {
                                obj("ok" to "true").toString()
                            }
                        }
                        else -> if (request.method == HttpMethod.Get) "[]" else "{}"
                    }
                respond(body, headers = headersOf(HttpHeaders.ContentType, "application/json"))
            }
        )
    val config =
        Config(
            options.text("origin").ifEmpty { "https://catalog.invalid" },
            "",
            options.nested("fixture").text("currentUserId").ifEmpty { "alice" },
            options.nested("fixture").flag("isAdmin"),
            txidMode = "none",
            assetsBaseUrl = options.text("origin"),
        )
    fun environment() =
        Environment(
            path =
                options.text("path").ifEmpty {
                    when (options.text("sample")) {
                        "settings-members" -> "/settings?tab=members"
                        "settings" -> "/settings"
                        "bubble" -> "/bubble"
                        else -> "/"
                    }
                },
            dark = options.flag("dark"),
            touch = options.flag("touch"),
            mac = options.flag("mac"),
            locale = options.text("locale"),
            visible = options.flag("visible"),
            focused =
                if (options.nested("fixture").containsKey("focused"))
                    options.nested("fixture").flag("focused")
                else options.flag("focused"),
            reducedMotion = options.flag("reducedMotion"),
            theme = options.text("theme").ifEmpty { if (options.flag("dark")) "dark" else "light" },
            historyState = options.nested("historyState"),
        )
    app = Session(Repository(config, platform, http, scope), scope, environment())
    refresh = { app.update(environment()) }
    var active by mutableStateOf(true)
    ComposeViewport(document.getElementById("zoo-compose-root")!!) {
        val fontsReady = zooFontsReady()
        if (active && fontsReady) {
            ZooTheme(
                options.flag("dark"),
                viewportWidth = options.text("viewportWidth").toFloatOrNull(),
                locale = environment().locale,
            ) {
                CompositionLocalProvider(
                    LocalApp provides app,
                    LocalWebPlatform provides platform,
                    LocalFrontendMode provides options.text("frontend").ifEmpty { "react" },
                ) {
                    Catalog(app, options)
                }
            }
            LaunchedEffect(Unit) { catalogPainted(CatalogSamples.message) }
        }
    }
    catalogReady(
        { raw ->
            val incoming = json.parseToJsonElement(raw).jsonObject
            val incomingPath = incoming.text("path")
            options =
                if (incomingPath == hostPath) {
                    JsonObject(
                        incoming + options.filterKeys { it == "path" || it == "historyState" }
                    )
                } else {
                    hostPath = incomingPath
                    navigationHistory.clear()
                    navigationIndex = 0
                    incoming
                }
            refresh()
        },
        {
            active = false
            app.close()
            scope.cancel()
        },
        { app.escape() },
    )
}
