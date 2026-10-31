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

@JsFun("() => JSON.stringify(globalThis.zooCatalog.options)")
private external fun catalogOptions(): String

@JsFun("(update, close) => globalThis.zooCatalog.ready(update, close)")
private external fun catalogReady(update: (String) -> Unit, close: () -> Unit)

@JsFun("(message) => globalThis.zooCatalog.painted(message)")
private external fun catalogPainted(message: String)

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
    val platform =
        object : Platform {
            override fun controlSemantics(
                id: String,
                role: String,
                enabled: Boolean,
                selected: Boolean?,
            ) = webControl(id, role, enabled, selected)

            override fun forgetControlSemantics(id: String) = forgetWebControl(id)

            override suspend fun headers() = emptyMap<String, String>()

            override suspend fun readCache(key: String): String? =
                when {
                    key.contains(":channels:") ->
                        (options.nested("fixture")["channels"]
                                ?: JsonArray(listOf(CatalogSamples.channel)))
                            .toString()
                    key.contains(":users:") ->
                        (options.nested("fixture")["users"] ?: JsonArray(CatalogSamples.users))
                            .toString()
                    else -> "[]"
                }

            override suspend fun writeCache(key: String, value: String) {}

            override fun uuid() = "catalog-${++sequence}"

            override fun now() = "2026-01-02T09:00:00Z"

            override fun navigate(path: String) {
                options = JsonObject(options + ("path" to JsonPrimitive(path)))
                refresh()
            }

            override fun action(name: String) {
                when {
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
                refresh()
            }

            override fun time(timestamp: String) = "09:00"

            override fun day(timestamp: String) = "2026-01-02"

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
            "https://catalog.invalid",
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
            locale = options.text("locale"),
            visible = options.text("sample") == "bubble",
            focused = false,
            reducedMotion = true,
            theme = options.text("theme").ifEmpty { if (options.flag("dark")) "dark" else "light" },
        )
    app = Session(Repository(config, platform, http, scope), scope, environment())
    refresh = { app.update(environment()) }
    var active by mutableStateOf(true)
    ComposeViewport(document.getElementById("zoo-compose-root")!!) {
        val fontsReady = zooFontsReady()
        if (active && fontsReady) {
            ZooTheme(options.flag("dark")) {
                CompositionLocalProvider(
                    LocalApp provides app,
                    LocalWebPlatform provides platform,
                ) {
                    Catalog(app, options)
                }
            }
            LaunchedEffect(Unit) { catalogPainted(CatalogSamples.message) }
        }
    }
    catalogReady(
        { raw ->
            options = json.parseToJsonElement(raw).jsonObject
            refresh()
        },
        {
            active = false
            app.close()
            scope.cancel()
        },
    )
}
