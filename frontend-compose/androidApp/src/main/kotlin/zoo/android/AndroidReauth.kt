package zoo.android

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.browser.auth.AuthTabIntent
import androidx.browser.customtabs.CustomTabsClient
import androidx.browser.customtabs.CustomTabsIntent
import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.request.forms.submitForm
import io.ktor.client.request.get
import io.ktor.client.statement.bodyAsText
import io.ktor.http.URLProtocol
import io.ktor.http.Url
import io.ktor.http.parameters
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import zoo.crypto.ReauthCallback
import zoo.crypto.ReauthCallbacks
import zoo.crypto.ReauthException
import zoo.crypto.ReauthRequest
import zoo.crypto.validateIdToken
import zoo.json

internal class BrowserAuthorization(val tokenEndpoint: String, val tokens: JsonObject) {
    override fun toString(): String = "BrowserAuthorization"
}

/** OIDC authorization code flow shared by sign-in and E2EE reauthentication. */
internal class AndroidReauth(
    private val activity: Activity,
    private val http: HttpClient,
    private val connection: LoginConnection,
    private val launchAuthTab: (AuthTabIntent, Uri, Uri, (AuthTabIntent.AuthResult) -> Unit) -> Boolean,
    private val readDiscovery: suspend (String) -> JsonObject = { readOidcDiscovery(it) },
) {
    private class Pending(val request: ReauthRequest, val result: CompletableDeferred<ReauthCallback>)

    private val pending = AtomicReference<Pending?>(null)

    /** App Links で受け取った URI を渡す。待っている再認証の `state` と合えば真。 */
    fun deliver(uri: String): Boolean {
        val current = pending.get() ?: return false
        val callback = ReauthCallbacks.parse(uri, current.request) ?: return false
        return current.result.complete(callback)
    }

    suspend fun reauthenticate(): String =
        requireNotNull(authorize(forceReauth = true).tokens.text("id_token"))

    suspend fun signIn(stage: (LoginStage) -> Unit): BrowserAuthorization = authorize(forceReauth = false, stage = stage)

    private suspend fun authorize(forceReauth: Boolean, stage: (LoginStage) -> Unit = {}): BrowserAuthorization {
        stage(LoginStage.Discovery)
        val discovery = discover()
        val request =
            ReauthRequest.create(
                authorizationEndpoint = discovery.text("authorization_endpoint")
                    ?: throw ReauthException(ReauthException.Reason.DENIED, "この IdP は認可コードフローに対応していません"),
                clientId = connection.client,
                // 接続先ではなく、manifest の App Links と同じビルド時のホストにする（独自の接続先でもアプリに戻る）。
                redirectUri = ReauthRequest.redirectUri("https://${works.zli.zoo.BuildConfig.REAUTH_HOST}"),
                resource = connection.resource,
                forceReauth = forceReauth,
            )
        val waiting = Pending(request, CompletableDeferred())
        check(pending.compareAndSet(null, waiting)) { "認証はすでに進行中です" }
        try {
            stage(LoginStage.OpenBrowser)
            withContext(Dispatchers.Main) {
                try {
                    val url = Uri.parse(request.url)
                    if (!openAuthTab(url, waiting)) {
                        CustomTabsIntent.Builder().setShareState(CustomTabsIntent.SHARE_STATE_OFF).build()
                            .launchUrl(activity, url)
                    }
                } catch (failure: ActivityNotFoundException) {
                    throw ReauthException(ReauthException.Reason.BROWSER_UNAVAILABLE, "ブラウザーを開けません")
                }
            }
            stage(LoginStage.BrowserCallback)
            val code =
                when (val callback =
                    withTimeoutOrNull(ReauthRequest.TIMEOUT_MILLIS) { waiting.result.await() }
                        ?: throw ReauthException(ReauthException.Reason.TIMEOUT, "認証が時間内に終わりませんでした")) {
                    is ReauthCallback.Code -> callback.code
                    is ReauthCallback.Error -> throw ReauthException(ReauthException.Reason.DENIED, "認証が取り消されました")
                }
            stage(LoginStage.Exchange)
            return exchange(discovery, request, code)
        } finally {
            pending.compareAndSet(waiting, null)
        }
    }

    private fun openAuthTab(url: Uri, waiting: Pending): Boolean {
        val browser = CustomTabsClient.getPackageName(activity, null) ?: return false
        if (!CustomTabsClient.isAuthTabSupported(activity, browser)) return false
        val tab = AuthTabIntent.Builder().build().apply { intent.setPackage(browser) }
        return try {
            launchAuthTab(tab, url, Uri.parse(waiting.request.redirectUri)) { result ->
                if (pending.get() === waiting) {
                    try {
                        waiting.result.complete(authTabCallback(result.resultCode, result.resultUri?.toString(), waiting.request))
                    } catch (failure: ReauthException) {
                        waiting.result.completeExceptionally(failure)
                    }
                }
            }
        } catch (_: ActivityNotFoundException) {
            false
        }
    }

    private suspend fun discover(): JsonObject {
        val issuer = connection.issuer
        require(Url(issuer).protocol == URLProtocol.HTTPS) { "認証先は HTTPS を指定してください" }
        val discovery = readDiscovery(issuer)
        if (discovery.text("issuer")?.trimEnd('/') != issuer.trimEnd('/')) {
            throw ReauthException(ReauthException.Reason.DENIED, "issuer が一致しません")
        }
        return discovery
    }

    private suspend fun exchange(discovery: JsonObject, request: ReauthRequest, code: String): BrowserAuthorization {
        val endpoint = discovery.text("token_endpoint")
        if (endpoint == null || Url(endpoint).protocol != URLProtocol.HTTPS) {
            throw ReauthException(ReauthException.Reason.DENIED, "トークンエンドポイントがありません")
        }
        val response =
            http.submitForm(
                endpoint,
                parameters {
                    append("grant_type", "authorization_code")
                    append("code", code)
                    append("redirect_uri", request.redirectUri)
                    append("code_verifier", request.verifier)
                    append("client_id", connection.client)
                    if (connection.resource.isNotBlank()) append("resource", connection.resource)
                },
            )
        val result =
            try {
                json.parseToJsonElement(response.bodyAsText()) as JsonObject
            } catch (_: Exception) {
                JsonObject(emptyMap())
            }
        // エラー応答の本文や成功時のトークンはログに出さない。
        val idToken =
            result.text("id_token")?.takeIf { response.status.value in 200..299 }
                ?: throw ReauthException(ReauthException.Reason.DENIED, "ID トークンを受け取れませんでした (HTTP ${response.status.value})")
        validateIdToken(idToken, connection.issuer, connection.client, request.nonce, System.currentTimeMillis() / 1000)
        return BrowserAuthorization(endpoint, result)
    }
}

// Discovery is read-only. Allow another IP address when the first network route fails.
internal suspend fun readOidcDiscovery(
    issuer: String,
    transport: okhttp3.OkHttpClient.Builder.() -> Unit = {},
): JsonObject = HttpClient(OkHttp) {
    followRedirects = false
    engine {
        config {
            retryOnConnectionFailure(true)
            followRedirects(false)
            followSslRedirects(false)
            transport()
        }
    }
    install(HttpTimeout) { requestTimeoutMillis = 30_000 }
}.use { client ->
    val response = client.get("${issuer.trimEnd('/')}/.well-known/openid-configuration")
    if (response.status.value !in 200..299) {
        throw ReauthException(ReauthException.Reason.DENIED,
            "認証先の設定を取得できません (HTTP ${response.status.value})")
    }
    response.loginJson("認証先の設定取得")
}

internal fun authTabCallback(code: Int, uri: String?, request: ReauthRequest): ReauthCallback =
    when (code) {
        AuthTabIntent.RESULT_OK -> uri?.let { ReauthCallbacks.parse(it, request) }
            ?: throw ReauthException(ReauthException.Reason.INVALID_CALLBACK, "認証の戻り先を確認できませんでした")
        AuthTabIntent.RESULT_VERIFICATION_FAILED, AuthTabIntent.RESULT_VERIFICATION_TIMED_OUT ->
            throw ReauthException(ReauthException.Reason.CALLBACK_VERIFICATION_FAILED, "認証の戻り先をブラウザーで検証できませんでした")
        else -> throw ReauthException(ReauthException.Reason.CANCELLED, "認証が取り消されました")
    }

private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull

/**
 * App Links（`https://zli.chat/android/reauth-callback`）の受け口。受け取った URI を [MainActivity] に渡して
 * すぐ閉じる。[MainActivity] は `FLAG_ACTIVITY_CLEAR_TOP` で上に重なった Custom Tabs を閉じ、
 * `FLAG_ACTIVITY_SINGLE_TOP` で今のインスタンス（メモリーの鍵を持つ画面）のまま `onNewIntent` で受け取る。
 */
class ReauthCallbackActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        intent?.data?.let { uri ->
            startActivity(
                Intent(this, MainActivity::class.java)
                    .setAction(ACTION_REAUTH_CALLBACK)
                    .setData(uri)
                    .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            )
        }
        finish()
    }

    companion object {
        const val ACTION_REAUTH_CALLBACK = "works.zli.zoo.REAUTH_CALLBACK"
    }
}
