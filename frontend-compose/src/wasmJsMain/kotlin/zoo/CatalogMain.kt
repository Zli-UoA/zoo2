@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import androidx.compose.runtime.*
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.window.ComposeViewport
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlin.JsFun
import kotlinx.browser.document
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

@JsFun("(boot) => { globalThis.__zooComposeCatalogBoot = boot; globalThis.dispatchEvent(new Event('zoo-compose-catalog-module-ready')); }")
private external fun catalogBoot(boot: () -> Unit)
@JsFun("() => JSON.stringify(globalThis.zooCatalog.options)")
private external fun catalogOptions(): String
@JsFun("(update, close) => globalThis.zooCatalog.ready(update, close)")
private external fun catalogReady(update: (String) -> Unit, close: () -> Unit)
@JsFun("(message) => globalThis.zooCatalog.painted(message)")
private external fun catalogPainted(message: String)

internal fun registerCatalog() = catalogBoot(::startCatalog)

@OptIn(ExperimentalComposeUiApi::class)
private fun startCatalog() {
    org.jetbrains.compose.resources.configureWebResources { resourcePathMapping { "/compose/$it" } }
    var options by mutableStateOf(json.parseToJsonElement(catalogOptions()).jsonObject)
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val storage = mutableMapOf<String, String>()
    var sequence = 0
    val platform = object : Platform {
        override suspend fun headers() = emptyMap<String, String>()
        override suspend fun readCache(key: String): String? = null
        override suspend fun writeCache(key: String, value: String) {}
        override fun uuid() = "catalog-${++sequence}"
        override fun now() = "2026-01-02T09:00:00Z"
        override fun navigate(path: String) {}
        override fun action(name: String) {}
        override fun time(timestamp: String) = "09:00"
        override fun day(timestamp: String) = "2026-01-02"
        override fun translate(key: String, args: Map<String, String>): String {
            var value = options.nested("labels").text(key).ifEmpty { key }
            args.forEach { (name, text) -> value = value.replace("{{$name}}", text) }
            return value
        }
        override suspend fun invoke(name: String, payload: String): String = when (name) {
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
    val http = HttpClient(MockEngine { request ->
        respond(if (request.method == HttpMethod.Get) "[]" else "{}",
            headers = headersOf(HttpHeaders.ContentType, "application/json"))
    })
    val config = Config("https://catalog.invalid", "", "alice", false,
        txidMode = "none", assetsBaseUrl = options.text("origin"))
    fun environment() = Environment(dark = options.flag("dark"), locale = options.text("locale"),
        visible = false, reducedMotion = true)
    val app = Session(Repository(config, platform, http, scope), scope, environment())
    var active by mutableStateOf(true)
    ComposeViewport(document.getElementById("zoo-compose-root")!!) {
        val fontsReady = zooFontsReady()
        if (active && fontsReady) {
            ZooTheme(options.flag("dark")) {
                CompositionLocalProvider(LocalApp provides app) { Catalog(app, options) }
            }
            LaunchedEffect(Unit) { catalogPainted(CatalogSamples.message) }
        }
    }
    catalogReady({ raw ->
        options = json.parseToJsonElement(raw).jsonObject
        app.env = environment()
    }, {
        active = false
        app.close()
        scope.cancel()
    })
}
