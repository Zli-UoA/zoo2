package zoo.android

import io.ktor.client.*
import io.ktor.client.network.sockets.ConnectTimeoutException
import io.ktor.client.request.*
import io.ktor.client.request.forms.*
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*
import zoo.*
import java.net.ConnectException
import java.net.UnknownHostException
import kotlinx.coroutines.CancellationException

/** Public-client token holder. Sign-in is the browser authorization code flow (PKCE, see AndroidReauth). Access tokens stay in memory; refresh persistence is owned by the app. */
internal class DeviceAuth(
    private val http: HttpClient,
    private val persistRefresh: (String?) -> Unit = {},
    private val consumeRefresh: () -> Unit = { persistRefresh(null) },
    private val readDiscovery: (suspend (String) -> Row)? = null,
    private val onExpired: (DeviceAuth) -> Unit = {},
) {
    private var token = ""
    private var refresh = ""
    private var expires = 0L
    private var endpoint = ""
    private var clientId = ""
    private var resource = ""
    private val lock = Mutex()
    private val persistenceLock = Any()
    private var closed = false

    private suspend fun discover(issuer: String, client: String, audience: String): Row {
        if (Url(issuer).protocol != URLProtocol.HTTPS) {
            throw LoginSettingsInvalid("認証先は HTTPS を指定してください") { it.t("android.login.issuerHttps") }
        }
        clientId = client
        resource = audience
        val discovery = readDiscovery?.invoke(issuer) ?: run {
            val response = http.get("${issuer.trimEnd('/')}/.well-known/openid-configuration")
            if (response.status.value !in 200..299) {
                val status = response.status.value
                throw LoginFailure("認証先の設定を取得できません (HTTP $status)") {
                    it.t("android.login.discoveryFailed", "status" to "$status")
                }
            }
            response.loginJson(LoginStep.Discovery)
        }
        if (discovery.text("issuer").trimEnd('/') != issuer.trimEnd('/')) {
            throw LoginSettingsInvalid("issuer が一致しません") { it.t("android.login.issuerMismatch") }
        }
        endpoint = discovery.text("token_endpoint")
        require(Url(endpoint).protocol == URLProtocol.HTTPS)
        return discovery
    }

    suspend fun acceptBrowserLogin(connection: LoginConnection, grant: BrowserAuthorization) =
        lock.withLock {
            require(Url(grant.tokenEndpoint).protocol == URLProtocol.HTTPS)
            clientId = connection.client
            resource = connection.resource
            endpoint = grant.tokenEndpoint
            accept(grant.tokens)
        }

    suspend fun restore(issuer: String, client: String, audience: String, savedRefresh: String, stage: (LoginStage) -> Unit = {}) =
        lock.withLock {
            require(savedRefresh.isNotBlank())
            stage(LoginStage.Discovery)
            discover(issuer, client, audience)
            stage(LoginStage.Restore)
            refresh = savedRefresh
            renew()
        }

    /** Serialize deletion with token rotation so logout cannot be undone by a late response. */
    fun forget() = synchronized(persistenceLock) {
        close()
        persistRefresh(null)
    }

    fun close() = synchronized(persistenceLock) {
        closed = true
        token = ""
        refresh = ""
    }

    private suspend fun renew() {
        currentCoroutineContext().ensureActive()
        withContext(NonCancellable) {
            // Once sent, a single-use refresh token must never be restored from disk.
            // Commit its removal before sending. Cancellation still waits for rotation
            // and persistence; timeout, malformed responses and save failure require login.
            val usedRefresh = synchronized(persistenceLock) {
                check(!closed) { "認証セッションは終了しています" }
                consumeRefresh()
                refresh
            }
            var receivedResponse = false
            try {
                val result = exchange(mapOf("grant_type" to "refresh_token", "refresh_token" to usedRefresh))
                receivedResponse = true
                accept(result)
            } catch (failure: Exception) {
                // The Android client disables automatic retries and redirects. Only a
                // connection failure before any response is safe to retry with this token.
                if (!receivedResponse && failure.isConnectionFailure()) {
                    val restored = runCatching {
                        synchronized(persistenceLock) {
                            if (closed) false else {
                                persistRefresh(usedRefresh)
                                refresh = usedRefresh
                                true
                            }
                        }
                    }.getOrDefault(false)
                    if (restored) throw LoginFailure("認証先に接続できません。通信状態を確認して再試行してください。", cause = failure) {
                        it.t("android.login.refreshUnreachable")
                    }
                }
                var notify = false
                try {
                    synchronized(persistenceLock) {
                        if (!closed) {
                            notify = true
                            forget()
                        }
                    }
                } finally {
                    if (notify) onExpired(this@DeviceAuth)
                }
                throw LoginFailure(
                    if (failure is LoginFailure) failure.message.orEmpty()
                    else "トークン更新の結果を確認できませんでした。ログインし直してください。",
                    requiresLogin = true,
                    cause = failure,
                    display = if (failure is LoginFailure) failure.display else ({ text: LoginText -> text.t("android.login.refreshUnconfirmed") }),
                )
            }
        }
    }

    private suspend fun exchange(fields: Map<String, String>): Row {
        val response =
            http.submitForm(
                endpoint,
                parameters {
                    append("client_id", clientId)
                    if (resource.isNotBlank()) append("resource", resource)
                    fields.forEach { (k, v) -> append(k, v) }
                },
            )
        val result = response.loginJson(LoginStep.Token)
        val error = result.text("error")
        if (response.status.value !in 200..299 || error.isNotEmpty()) {
            throw oauthLoginFailure(LoginStep.Token, response.status.value, error)
        }
        return result
    }

    private suspend fun accept(result: Row) {
        currentCoroutineContext().ensureActive()
        synchronized(persistenceLock) {
            check(!closed) { "認証セッションは終了しています" }
            if (result.present("error") || result.text("access_token").isBlank()) {
                throw LoginFailure("認証サーバーからトークンを取得できませんでした。ログインし直してください") {
                    it.t("android.login.noAccessToken")
                }
            }
            token = result.text("access_token")
            refresh = result.text("refresh_token").ifEmpty { refresh }
            expires =
                System.currentTimeMillis() + (result.text("expires_in").toLongOrNull() ?: 300) * 1000
            persistRefresh(refresh.takeIf { it.isNotBlank() })
        }
    }

    /** Cleanup must never start or wait for refresh-token rotation during logout. */
    fun headersWithoutRefresh(): Map<String, String> = synchronized(persistenceLock) {
        check(!closed && token.isNotBlank()) { "認証セッションは終了しています" }
        mapOf("Authorization" to "Bearer $token")
    }

    suspend fun headers(): Map<String, String> =
        lock.withLock {
            check(!synchronized(persistenceLock) { closed }) { "認証セッションは終了しています" }
            if (System.currentTimeMillis() >= expires - 60000) {
                check(refresh.isNotBlank()) { "ログインし直してください" }
                renew()
            }
            mapOf("Authorization" to "Bearer $token")
        }
}

/** 表示言語の翻訳（`AppTranslator.t` など）。 */
internal class LoginText(private val translate: (String, Array<out Pair<String, String>>) -> String) {
    fun t(key: String, vararg args: Pair<String, String>): String = translate(key, args)
}

internal fun android.content.Context.loginText(): LoginText {
    val translator = AppTranslator(this)
    return LoginText { key, args -> translator.t(key, *args) }
}

/**
 * Only locally constructed messages are safe to show in the login screen.
 * `message` は日本語のまま（診断ログの分類と HTTP 状態の取り出し、テストに使う）。画面には [display] で表示言語の文言を出す。
 */
internal class LoginFailure(
    message: String,
    val requiresLogin: Boolean = false,
    cause: Throwable? = null,
    val display: (LoginText) -> String = { message },
) : IllegalStateException(message, cause)

/** 接続先の設定や応答の形が正しくない（診断ログでは INVALID_RESPONSE_OR_SETTINGS）。表示は [display]。 */
internal class LoginSettingsInvalid(message: String, val display: (LoginText) -> String) : IllegalArgumentException(message)

/** ログインの段階（エラーの文言に入れる名前）。 */
internal enum class LoginStep(val label: String) {
    Discovery("認証先の設定取得"),
    Token("トークンの取得");

    fun text(text: LoginText): String = when (this) {
        Discovery -> text.t("android.login.step.discovery")
        Token -> text.t("android.login.step.token")
    }
}

internal suspend fun HttpResponse.loginJson(stage: LoginStep): Row {
    val raw = bodyAsText()
    return try {
        json.parseToJsonElement(raw).jsonObject
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        val code = status.value
        throw LoginFailure("${stage.label} に失敗しました (HTTP $code)。認証サーバーの応答を確認してください") {
            it.t("android.login.invalidResponse", "stage" to stage.text(it), "status" to "$code")
        }
    }
}

private fun oauthLoginFailure(stage: LoginStep, status: Int?, code: String): LoginFailure {
    val hint =
        when (code) {
            "unauthorized_client" -> "AuthZ で Public client の認可コードと refresh_token の認証を許可してください"
            "invalid_target" -> "Resource / audience と、AuthZ の API 設定・クライアントへの紐付けを確認してください"
            "invalid_scope" -> "openid profile offline_access の利用が許可されているか確認してください"
            "invalid_client" -> "Client ID と、クライアントが有効な Public client であることを確認してください"
            "access_denied" -> "ブラウザーでログインが拒否されました。もう一度ログインしてください"
                        "invalid_grant" -> "認証情報が無効か、期限が切れています。もう一度ログインしてください"
            else -> null
        }
    // Do not echo error_description, response bodies, or unrecognized server values.
    val details =
        listOfNotNull(status?.let { "HTTP $it" }, code.takeIf { hint != null }).joinToString(" / ")
    return LoginFailure(
        "${stage.label} に失敗しました ($details)。${hint ?: "認証サーバーの設定を確認してください"}",
        requiresLogin = status != null && status in 400..499 && code in setOf("invalid_grant", "invalid_client", "unauthorized_client", "access_denied", "expired_token"),
    ) { text ->
        val translatedHint = when (code.takeIf { hint != null }) {
            "unauthorized_client" -> text.t("android.login.oauthHint.unauthorizedClient")
            "invalid_target" -> text.t("android.login.oauthHint.invalidTarget")
            "invalid_scope" -> text.t("android.login.oauthHint.invalidScope")
            "invalid_client" -> text.t("android.login.oauthHint.invalidClient")
            "access_denied" -> text.t("android.login.oauthHint.accessDenied")
            "invalid_grant" -> text.t("android.login.oauthHint.invalidGrant")
            else -> text.t("android.login.oauthHint.checkServer")
        }
        text.t("android.login.stepFailed", "stage" to stage.text(text), "details" to details, "hint" to translatedHint)
    }
}

/** Read timeouts and generic IO failures may follow a successful token exchange. */
private fun Throwable.isConnectionFailure(): Boolean {
    var current: Throwable? = this
    val seen = mutableSetOf<Throwable>()
    while (current != null && seen.add(current)) {
        if (current is UnknownHostException || current is ConnectException || current is ConnectTimeoutException) return true
        current = current.cause
    }
    return false
}


internal const val wifiSignInMessage = "Wi-Fiのログインが必要です。AndroidのWi-Fi設定から接続を完了し、再試行してください。"

/** [loginNetworkMessage] の表示言語版（比較には日本語の [wifiSignInMessage] を使う）。 */
internal fun loginNetworkText(failure: Throwable, captivePortal: Boolean, text: LoginText): String? {
    if (captivePortal) return text.t("android.login.wifiSignIn")
    return when (loginNetworkIssue(failure)) {
        LoginNetworkIssue.Dns -> text.t("android.login.network.dns")
        LoginNetworkIssue.Certificate -> text.t("android.login.network.certificate")
        LoginNetworkIssue.Timeout -> text.t("android.login.network.timeout")
        LoginNetworkIssue.Connection, LoginNetworkIssue.Io -> text.t("android.login.network.connection")
        null -> null
    }
}

internal fun loginNetworkMessage(failure: Throwable, captivePortal: Boolean): String? {
    if (captivePortal) return wifiSignInMessage
    return when (loginNetworkIssue(failure)) {
        LoginNetworkIssue.Dns -> "接続先の名前を解決できません。Wi-Fiの接続状態を確認してください。"
        LoginNetworkIssue.Certificate -> "接続先の証明書を確認できません。端末の日時とWi-Fiの接続状態を確認してください。"
        LoginNetworkIssue.Timeout -> "接続先から時間内に応答がありませんでした。ネットワークを切り替えるか、再試行してください。"
        LoginNetworkIssue.Connection, LoginNetworkIssue.Io -> "接続先に接続できません。Wi-Fiの接続状態を確認してください。"
        null -> null
    }
}
