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
        require(Url(issuer).protocol == URLProtocol.HTTPS) { "認証先は HTTPS を指定してください" }
        clientId = client
        resource = audience
        val response = http.get("${issuer.trimEnd('/')}/.well-known/openid-configuration")
        if (response.status.value !in 200..299) {
            throw LoginFailure("認証先の設定を取得できません (HTTP ${response.status.value})")
        }
        val discovery = response.loginJson("認証先の設定取得")
        require(discovery.text("issuer").trimEnd('/') == issuer.trimEnd('/')) { "issuer が一致しません" }
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

    suspend fun restore(issuer: String, client: String, audience: String, savedRefresh: String) =
        lock.withLock {
            require(savedRefresh.isNotBlank())
            discover(issuer, client, audience)
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
                    if (restored) throw LoginFailure("認証先に接続できません。通信状態を確認して再試行してください。")
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
        val result = response.loginJson("トークンの取得")
        val error = result.text("error")
        if (response.status.value !in 200..299 || error.isNotEmpty()) {
            throw oauthLoginFailure("トークンの取得", response.status.value, error)
        }
        return result
    }

    private suspend fun accept(result: Row) {
        currentCoroutineContext().ensureActive()
        synchronized(persistenceLock) {
            check(!closed) { "認証セッションは終了しています" }
            if (result.present("error") || result.text("access_token").isBlank()) {
                throw LoginFailure("認証サーバーからトークンを取得できませんでした。ログインし直してください")
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

/** Only locally constructed messages are safe to show in the login screen. */
internal class LoginFailure(message: String, val requiresLogin: Boolean = false) : IllegalStateException(message)

internal suspend fun HttpResponse.loginJson(stage: String): Row {
    val raw = bodyAsText()
    return try {
        json.parseToJsonElement(raw).jsonObject
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        throw LoginFailure("$stage に失敗しました (HTTP ${status.value})。認証サーバーの応答を確認してください")
    }
}

private fun oauthLoginFailure(stage: String, status: Int?, code: String): LoginFailure {
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
        "$stage に失敗しました ($details)。${hint ?: "認証サーバーの設定を確認してください"}",
        requiresLogin = status != null && status in 400..499 && code in setOf("invalid_grant", "invalid_client", "unauthorized_client", "access_denied", "expired_token"),
    )
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
