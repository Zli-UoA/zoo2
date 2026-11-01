@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import androidx.compose.runtime.*
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.window.ComposeViewport
import io.ktor.client.*
import io.ktor.client.plugins.*
import io.ktor.client.plugins.api.*
import kotlin.JsFun
import kotlin.js.*
import kotlinx.browser.document
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import org.jetbrains.compose.resources.preloadFont
import zoo_compose.generated.resources.*

@JsFun("() => globalThis.__zooCompose") private external fun currentBridge(): JsAny

@JsFun("(bridge) => bridge.config") private external fun configJson(bridge: JsAny): String

@JsFun("(bridge) => bridge.headers()")
private external fun authHeaders(bridge: JsAny): Promise<JsString>

@JsFun("(bridge, key) => bridge.cache.read(key)")
private external fun cacheRead(bridge: JsAny, key: String): Promise<JsString?>

@JsFun("(bridge, key, value) => bridge.cache.write(key, value)")
private external fun cacheWrite(bridge: JsAny, key: String, value: String): Promise<JsAny?>

@JsFun("() => crypto.randomUUID()") private external fun randomId(): String

@JsFun("() => new Date().toISOString()") private external fun currentTime(): String

@JsFun("(bridge, path) => bridge.navigate(path)")
private external fun navigateTo(bridge: JsAny, path: String)

@JsFun("(bridge, path, state) => bridge.navigate(path, false, JSON.parse(state))")
private external fun navigateStateTo(bridge: JsAny, path: String, state: String)

@JsFun("(bridge, path) => bridge.navigate(path, true)")
private external fun replaceTo(bridge: JsAny, path: String)

@JsFun("(bridge, status) => bridge.authStatus(status)")
private external fun authStatus(bridge: JsAny, status: Int)

@JsFun("(bridge, key, args) => bridge.translate(key, args)")
private external fun translateText(bridge: JsAny, key: String, args: String): String

@JsFun("(bridge, name) => bridge.action(name)")
internal external fun browserAction(bridge: JsAny, name: String)

@JsFun("(bridge, name, payload) => bridge.invoke(name, payload)")
internal external fun browserInvoke(bridge: JsAny, name: String, payload: String): Promise<JsString>

@JsFun("(bridge, value) => bridge.notificationTime(value)")
private external fun browserNotificationTime(bridge: JsAny, value: String): String

@JsFun("(bridge, value) => bridge.fullTimestamp(value)")
private external fun browserFullTimestamp(bridge: JsAny, value: String): String

@JsFun("(bridge, value, day, offset) => bridge.format(value, day, offset)")
private external fun formatTime(bridge: JsAny, value: String, day: Boolean, offset: Int): String

@JsFun("(bridge) => bridge.paintReady()") private external fun paintReady(bridge: JsAny)

@JsFun("(bridge, update, dispose, escape) => bridge.ready(update, dispose, escape)")
private external fun ready(
    bridge: JsAny,
    update: (String) -> Unit,
    dispose: () -> Unit,
    escape: () -> Boolean,
)

/** JS failures must enter the Kotlin Exception hierarchy so feature retries can handle them. */
internal suspend fun <T : JsAny?> Promise<T>.awaitBrowser(): T =
    try {
        await()
    } catch (error: JsException) {
        throw IllegalStateException(error.message, error)
    }

private class BrowserPlatform(private val bridge: JsAny) : Platform {
    private val files = BrowserFileBridge(bridge)

    override fun imageFrames(bytes: ByteArray) = webImageFrames(bytes)

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

    @OptIn(kotlin.io.encoding.ExperimentalEncodingApi::class)
    override suspend fun image(url: String): ByteArray =
        kotlin.io.encoding.Base64.decode(invoke("publicImage", url))

    override suspend fun rasterizeSvg(svg: String, width: Int, height: Int): ByteArray =
        browserRasterizeSvg(svg, width, height)

    override suspend fun headers(): Map<String, String> =
        json.decodeFromString(authHeaders(bridge).awaitBrowser<JsString>().toString())

    override suspend fun readCache(key: String) =
        cacheRead(bridge, key).awaitBrowser<JsString?>()?.toString()

    override suspend fun writeCache(key: String, value: String) {
        cacheWrite(bridge, key, value).awaitBrowser<JsAny?>()
    }

    override fun uuid() = randomId()

    override fun now() = currentTime()

    override fun navigate(path: String) = navigateTo(bridge, path)

    override fun navigate(path: String, state: Row) =
        navigateStateTo(bridge, path, state.toString())

    override fun replace(path: String) = replaceTo(bridge, path)

    override fun translate(key: String, args: Map<String, String>) =
        translateText(bridge, key, json.encodeToString(args))

    override fun action(name: String) = browserAction(bridge, name)

    override suspend fun invoke(name: String, payload: String): String {
        if (name !in listOf("publicGet", "publicImage"))
            return browserInvoke(bridge, name, payload).awaitBrowser<JsString>().toString()
        val id = randomId()
        try {
            return browserInvoke(
                    bridge,
                    name,
                    obj("requestId" to id, "payload" to payload).toString(),
                )
                .awaitBrowser<JsString>()
                .toString()
        } finally {
            if (!currentCoroutineContext().isActive) browserInvoke(bridge, "cancelPublic", id)
        }
    }

    override fun time(timestamp: String) = formatTime(bridge, timestamp, false, 0)

    override fun notificationTime(timestamp: String) = browserNotificationTime(bridge, timestamp)

    override fun fullTimestamp(timestamp: String) = browserFullTimestamp(bridge, timestamp)

    override fun day(timestamp: String) = formatTime(bridge, timestamp, true, 0)

    override fun previousDay(timestamp: String) = formatTime(bridge, timestamp, true, -1)
}

private fun environment(raw: String): Environment {
    val r = json.parseToJsonElement(raw).jsonObject
    return Environment(
        r.text("path"),
        r.flag("online"),
        r.flag("visible"),
        r.flag("dark"),
        r.text("locale"),
        r.flag("mac"),
        r.flag("touch"),
        r.flag("oidc"),
        r.flag("reducedMotion"),
        r.flag("development"),
        r.flag("focused"),
        r.text("persistenceStatus"),
        r.text("persistenceError"),
        r.text("theme").ifEmpty { "system" },
        r.nested("historyState"),
    )
}

@JsFun(
    "(boot) => { globalThis.__zooComposeBoot = boot; globalThis.dispatchEvent(new Event('zoo-compose-module-ready')); }"
)
private external fun registerBoot(boot: () -> Unit)

fun main() {
    registerBoot(::startSession)
    registerCatalog()
}

@OptIn(
    ExperimentalComposeUiApi::class,
    org.jetbrains.compose.resources.ExperimentalResourceApi::class,
)
private fun startSession() {
    org.jetbrains.compose.resources.configureWebResources {
        resourcePathMapping { path -> "/compose/$path" }
    }
    val bridge = currentBridge()
    val raw = json.parseToJsonElement(configJson(bridge)).jsonObject
    val config =
        Config(
            raw.text("apiBaseUrl"),
            raw.text("shapeProxyPath"),
            raw.text("userId"),
            raw.flag("isAdmin"),
            raw.text("shapeMode"),
            raw.text("electricShapeUrl"),
            raw.text("txidMode"),
            raw.text("assetsBaseUrl"),
            raw["previewUsers"]?.rows().orEmpty(),
            raw.text("summalyUrl"),
            raw.text("mediaProxyUrl"),
        )
    val http = HttpClient {
        expectSuccess = false
        install(HttpTimeout) {
            requestTimeoutMillis = 120_000
            socketTimeoutMillis = 90_000
        }
        install(
            createClientPlugin("ZooAuthentication") {
                onResponse { authStatus(bridge, it.status.value) }
            }
        )
    }
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val platform = BrowserPlatform(bridge)
    var envRaw by mutableStateOf(raw.text("environment"))
    val status = json.parseToJsonElement(envRaw).jsonObject.text("status")
    val app =
        if (status.isEmpty())
            Session(Repository(config, platform, http, scope), scope, environment(envRaw))
        else null
    var active by mutableStateOf(true)
    ComposeViewport(document.getElementById("zoo-compose-root")!!) {
        // Public font requests must outlive this session's visible content: a loading
        // session may be replaced while a new session awaits the same cached request.
        // The Web resource API returns an empty placeholder until each font is ready.
        // Keep the bootstrap visible until every family and weight used by the theme loads.
        val fontsReady = zooFontsReady()
        if (active && fontsReady) {
            LaunchedEffect(Unit) { paintReady(bridge) }
            CompositionLocalProvider(LocalWebPlatform provides platform) {
                ZooTheme(environment(envRaw).dark, locale = environment(envRaw).locale) {
                    if (app != null)
                        CompositionLocalProvider(LocalApp provides app) {
                            NativeWindow(app) { ZooApp(app) }
                        }
                    else NativeStatus(platform, json.parseToJsonElement(envRaw).jsonObject)
                }
            }
        }
    }
    app?.start()
    ready(
        bridge,
        {
            envRaw = it
            app?.update(environment(it))
        },
        {
            active = false
            app?.close()
            http.close()
            scope.cancel()
        },
        { app?.escape() ?: false },
    )
}

@OptIn(org.jetbrains.compose.resources.ExperimentalResourceApi::class)
@Composable
internal fun zooFontsReady(): Boolean {
    val fonts =
        listOf(
            preloadFont(Res.font.Geist).value,
            preloadFont(Res.font.Geist, FontWeight.Medium).value,
            preloadFont(Res.font.Geist, FontWeight.SemiBold).value,
            preloadFont(Res.font.NotoSansJPRegular).value,
            preloadFont(Res.font.NotoSansJPRegular, FontWeight.Medium).value,
            preloadFont(Res.font.NotoSansJPBold, FontWeight.SemiBold).value,
            preloadFont(Res.font.NotoColorEmoji).value,
            preloadFont(Res.font.LiberationMono).value,
            preloadFont(Res.font.KaTeXMain).value,
            preloadFont(Res.font.KaTeXMath).value,
        )
    return fonts.all { it != null }
}
