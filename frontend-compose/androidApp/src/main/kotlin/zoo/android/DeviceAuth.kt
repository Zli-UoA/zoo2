package zoo.android

import io.ktor.client.*
import io.ktor.client.request.*
import io.ktor.client.request.forms.*
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*
import zoo.*

/** Public-client device grant. Tokens stay in memory and never enter backups or preferences. */
internal class DeviceAuth(private val http: HttpClient) {
    private var token = ""
    private var refresh = ""
    private var expires = 0L
    private var endpoint = ""
    private var clientId = ""
    private var resource = ""
    private val lock = Mutex()

    suspend fun login(
        issuer: String,
        client: String,
        audience: String,
        show: (String, String) -> Unit,
    ) {
        require(Url(issuer).protocol == URLProtocol.HTTPS) { "認証先は HTTPS を指定してください" }
        clientId = client
        resource = audience
        val discoveryResponse = http.get("${issuer.trimEnd('/')}/.well-known/openid-configuration")
        check(discoveryResponse.status.value in 200..299) { "認証先の設定を取得できません" }
        val discovery = json.parseToJsonElement(discoveryResponse.bodyAsText()).jsonObject
        require(discovery.text("issuer").trimEnd('/') == issuer.trimEnd('/')) { "issuer が一致しません" }
        endpoint = discovery.text("token_endpoint")
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
        check(response.status.value in 200..299) { "ログインを開始できません。クライアントの設定を確認してください" }
        val device = json.parseToJsonElement(response.bodyAsText()).jsonObject
        val link =
            device.text("verification_uri_complete").ifEmpty { device.text("verification_uri") }
        require(Url(link).protocol == URLProtocol.HTTPS)
        show(device.text("user_code"), link)
        val deadline = System.currentTimeMillis() + device.text("expires_in").toLong() * 1000
        var interval = (device.text("interval").toLongOrNull() ?: 5).coerceAtLeast(1) * 1000
        while (System.currentTimeMillis() < deadline) {
            delay(interval)
            check(System.currentTimeMillis() < deadline) { "コードの期限が切れました" }
            val result =
                exchange(
                    mapOf(
                        "grant_type" to "urn:ietf:params:oauth:grant-type:device_code",
                        "device_code" to device.text("device_code"),
                    )
                )
            when (result.text("error")) {
                "" -> {
                    accept(result)
                    return
                }
                "authorization_pending" -> Unit
                "slow_down" -> interval += 5000
                else -> error("ログインが拒否されたか、コードの期限が切れました")
            }
        }
        error("コードの期限が切れました")
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
        val result = json.parseToJsonElement(response.bodyAsText()).jsonObject
        check(response.status.value in 200..299 || result.present("error")) { "認証サーバーに接続できません" }
        return result
    }

    private fun accept(result: Row) {
        check(!result.present("error") && result.present("access_token")) { "ログインし直してください" }
        token = result.text("access_token")
        refresh = result.text("refresh_token").ifEmpty { refresh }
        expires =
            System.currentTimeMillis() + (result.text("expires_in").toLongOrNull() ?: 300) * 1000
    }

    suspend fun headers(): Map<String, String> =
        lock.withLock {
            if (System.currentTimeMillis() >= expires - 60000) {
                check(refresh.isNotBlank()) { "ログインし直してください" }
                accept(exchange(mapOf("grant_type" to "refresh_token", "refresh_token" to refresh)))
            }
            mapOf("Authorization" to "Bearer $token")
        }
}
