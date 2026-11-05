package zoo.android

import io.ktor.client.*
import io.ktor.client.request.*
import io.ktor.client.request.forms.*
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*
import zoo.*

/** Public-client device grant. Access tokens stay in memory; refresh persistence is owned by the app. */
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
            try {
                accept(exchange(mapOf("grant_type" to "refresh_token", "refresh_token" to usedRefresh)))
            } catch (failure: Exception) {
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

    suspend fun login(
        issuer: String,
        client: String,
        audience: String,
        show: (String, String) -> Unit,
    ) {
        val discovery = discover(issuer, client, audience)
        val deviceEndpoint = discovery.text("device_authorization_endpoint")
        require(
            Url(endpoint).protocol == URLProtocol.HTTPS &&
                Url(deviceEndpoint).protocol == URLProtocol.HTTPS
        )
        val response =
            http.submitForm(
                deviceEndpoint,
                parameters {
                    append("client_id", client)
                    append("scope", "openid profile offline_access")
                    if (audience.isNotBlank()) append("resource", audience)
                },
            )
        val device = response.loginJson("確認コードの発行")
        if (response.status.value !in 200..299) {
            throw oauthLoginFailure("確認コードの発行", response.status.value, device.text("error"))
        }
        val link =
            device.text("verification_uri_complete").ifEmpty { device.text("verification_uri") }
        require(Url(link).protocol == URLProtocol.HTTPS)
        show(device.text("user_code"), link)
        val deadline = System.currentTimeMillis() + device.text("expires_in").toLong() * 1000
        var interval = (device.text("interval").toLongOrNull() ?: 5).coerceAtLeast(1) * 1000
        while (System.currentTimeMillis() < deadline) {
            delay(interval)
            if (System.currentTimeMillis() >= deadline) {
                throw LoginFailure("コードの期限が切れました。ログインし直してください")
            }
            val grant = mapOf(
                "grant_type" to "urn:ietf:params:oauth:grant-type:device_code",
                "device_code" to device.text("device_code"),
            )
            val result = exchange(grant)
            when (result.text("error")) {
                "" -> {
                    accept(result)
                    return
                }
                "authorization_pending" -> Unit
                "slow_down" -> interval += 5000
                else -> throw oauthLoginFailure("ログイン", null, result.text("error"))
            }
        }
        throw LoginFailure("コードの期限が切れました。ログインし直してください")
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
        val pending = error in setOf("authorization_pending", "slow_down")
        if (!pending && (response.status.value !in 200..299 || error.isNotEmpty())) {
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

private suspend fun HttpResponse.loginJson(stage: String): Row {
    val raw = bodyAsText()
    return try {
        json.parseToJsonElement(raw).jsonObject
    } catch (_: Exception) {
        throw LoginFailure("$stage に失敗しました (HTTP ${status.value})。認証サーバーの応答を確認してください")
    }
}

private fun oauthLoginFailure(stage: String, status: Int?, code: String): LoginFailure {
    val hint =
        when (code) {
            "unauthorized_client" -> "AuthZ で Public client のデバイスコード認証を許可してください"
            "invalid_target" -> "Resource / audience と、AuthZ の API 設定・クライアントへの紐付けを確認してください"
            "invalid_scope" -> "openid profile offline_access の利用が許可されているか確認してください"
            "invalid_client" -> "Client ID と、クライアントが有効な Public client であることを確認してください"
            "access_denied" -> "ブラウザーでログインが拒否されました。もう一度ログインしてください"
            "expired_token" -> "確認コードの期限が切れました。もう一度ログインしてください"
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
