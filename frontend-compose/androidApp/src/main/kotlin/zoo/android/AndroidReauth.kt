package zoo.android

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.browser.customtabs.CustomTabsIntent
import io.ktor.client.HttpClient
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

/**
 * E2EE の直前の再認証を Custom Tabs で行う（docs/e2ee.md の「クロス署名」の再認証の確かめ方。#360）。
 *
 * - 認可リクエスト（PKCE、`max_age=0`、`state`、`nonce`）を Custom Tabs で開く。今の画面（とメモリーの鍵、
 *   リカバリーキー）はそのまま残る。
 * - AuthZ のリダイレクト先 `https://zli.chat/android/reauth-callback` は App Links でこのアプリに届く
 *   （[ReauthCallbackActivity] から [MainActivity] へ渡し、[deliver] で受け取る）。`state` が違う URI は無視する。
 * - 認可コードはトークンエンドポイントで ID トークンに換え、iss、aud、exp、nonce を確かめる。今のセッションの
 *   トークンは変えない。
 *
 * 【未検証】AuthZ からの 302 のリダイレクトで、Custom Tabs がアプリに処理を渡すか（docs/e2ee.md の「未決と
 * 未検証の一覧」）。渡さなかったときは、ブラウザーに zoo2 の案内のページが出るので、「ログインをやめる」で戻る。
 */
internal class AndroidReauth(
    private val activity: Activity,
    private val http: HttpClient,
    private val connection: LoginConnection,
) {
    private class Pending(val request: ReauthRequest, val result: CompletableDeferred<ReauthCallback>)

    private val pending = AtomicReference<Pending?>(null)

    /** App Links で受け取った URI を渡す。待っている再認証の `state` と合えば真。 */
    fun deliver(uri: String): Boolean {
        val current = pending.get() ?: return false
        val callback = ReauthCallbacks.parse(uri, current.request) ?: return false
        return current.result.complete(callback)
    }

    suspend fun reauthenticate(): String {
        val discovery = discover()
        val request =
            ReauthRequest.create(
                authorizationEndpoint = discovery.text("authorization_endpoint")
                    ?: throw ReauthException(ReauthException.Reason.DENIED, "この IdP は認可コードフローに対応していません"),
                clientId = connection.client,
                redirectUri = ReauthRequest.redirectUri(connection.base),
                resource = connection.resource,
            )
        val waiting = Pending(request, CompletableDeferred())
        pending.set(waiting)
        try {
            withContext(Dispatchers.Main) {
                try {
                    CustomTabsIntent.Builder().setShareState(CustomTabsIntent.SHARE_STATE_OFF).build()
                        .launchUrl(activity, Uri.parse(request.url))
                } catch (failure: ActivityNotFoundException) {
                    throw ReauthException(ReauthException.Reason.BROWSER_UNAVAILABLE, "ブラウザーを開けません")
                }
            }
            val code =
                when (val callback =
                    withTimeoutOrNull(ReauthRequest.TIMEOUT_MILLIS) { waiting.result.await() }
                        ?: throw ReauthException(ReauthException.Reason.TIMEOUT, "再認証が時間内に終わりませんでした")) {
                    is ReauthCallback.Code -> callback.code
                    is ReauthCallback.Error -> throw ReauthException(ReauthException.Reason.DENIED, "再認証が拒否されました")
                }
            return exchange(discovery, request, code)
        } finally {
            pending.compareAndSet(waiting, null)
        }
    }

    private suspend fun discover(): JsonObject {
        val issuer = connection.issuer
        require(Url(issuer).protocol == URLProtocol.HTTPS) { "認証先は HTTPS を指定してください" }
        val response = http.get("${issuer.trimEnd('/')}/.well-known/openid-configuration")
        if (response.status.value !in 200..299) {
            throw ReauthException(ReauthException.Reason.DENIED, "認証先の設定を取得できません (HTTP ${response.status.value})")
        }
        val discovery = json.parseToJsonElement(response.bodyAsText()) as JsonObject
        if (discovery.text("issuer")?.trimEnd('/') != issuer.trimEnd('/')) {
            throw ReauthException(ReauthException.Reason.DENIED, "issuer が一致しません")
        }
        return discovery
    }

    private suspend fun exchange(discovery: JsonObject, request: ReauthRequest, code: String): String {
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
        // 成功応答の access_token は使わない（今のセッションのトークンは変えない）。エラーの本文は出さない。
        val idToken =
            result.text("id_token")?.takeIf { response.status.value in 200..299 }
                ?: throw ReauthException(ReauthException.Reason.DENIED, "ID トークンを受け取れませんでした (HTTP ${response.status.value})")
        validateIdToken(idToken, connection.issuer, connection.client, request.nonce, System.currentTimeMillis() / 1000)
        return idToken
    }
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
