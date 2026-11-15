package zoo.crypto

import io.ktor.http.URLBuilder
import io.ktor.http.Url
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull

/**
 * E2EE の直前の再認証（docs/e2ee.md の「初回設定と解錠の流れ」の 7 と「クロス署名」の API。#360）。
 *
 * Android はふだんのログインにデバイス認可グラントを使うので、`max_age` を付けた ID トークンを取れない。
 * 再認証のときだけ、認可コードフロー（PKCE の S256、`max_age=0`、`state`、`nonce`）を Custom Tabs で開き、
 * `https://zli.chat/android/reauth-callback` の App Links で認可コードを受け取って、トークンエンドポイントで
 * ID トークンに換える（TUI の `client-core` の `auth/reauth.rs` と同じ要求と検査）。今のセッションの
 * トークンは変えない。PKCE の code verifier と `nonce` はメモリーにだけ置く。
 */
class ReauthRequest private constructor(
    /** Custom Tabs で開く URL。 */
    val url: String,
    val redirectUri: String,
    val state: String,
    val nonce: String,
    val verifier: String,
) {
    override fun toString(): String = "ReauthRequest(redirectUri=$redirectUri)"

    companion object {
        /** リダイレクト先のパス。zoo2 のフロントエンドの nginx が App Links の `assetlinks.json` を配る。 */
        const val CALLBACK_PATH = "/android/reauth-callback"

        /** 利用者がログインし直すのを待つ時間（Web、TUI と同じ 10 分）。 */
        const val TIMEOUT_MILLIS = 10 * 60 * 1000L

        private val random = SecureRandom()
        private val encoder = Base64.getUrlEncoder().withoutPadding()

        /** 32 バイトの乱数の base64url（43 文字）。PKCE の verifier（43〜128 文字の unreserved）にも使える。 */
        fun randomToken(): String = encoder.encodeToString(ByteArray(32).also(random::nextBytes))

        /** PKCE の S256 の code challenge。 */
        fun codeChallenge(verifier: String): String =
            encoder.encodeToString(MessageDigest.getInstance("SHA-256").digest(verifier.encodeToByteArray()))

        /** `origin`（App Links のホストの `https://zli.chat` など。アプリのビルド時に決める）のリダイレクト先。 */
        fun redirectUri(origin: String): String = origin.trimEnd('/') + CALLBACK_PATH

        /**
         * 認可リクエストを作る。scope は `openid profile`（再認証では refresh token を作らないので
         * `offline_access` を外す）。`resource` はふだんのログインと同じ値（空なら付けない）。
         */
        fun create(
            authorizationEndpoint: String,
            clientId: String,
            redirectUri: String,
            resource: String,
            state: String = randomToken(),
            nonce: String = randomToken(),
            verifier: String = randomToken(),
        ): ReauthRequest {
            require(Url(authorizationEndpoint).protocol.name == "https") { "認可エンドポイントは HTTPS にしてください" }
            val url =
                URLBuilder(authorizationEndpoint).apply {
                    parameters.append("response_type", "code")
                    parameters.append("client_id", clientId)
                    parameters.append("redirect_uri", redirectUri)
                    parameters.append("scope", "openid profile")
                    if (resource.isNotBlank()) parameters.append("resource", resource)
                    parameters.append("state", state)
                    parameters.append("nonce", nonce)
                    parameters.append("code_challenge", codeChallenge(verifier))
                    parameters.append("code_challenge_method", "S256")
                    // 既存の AuthZ のセッションがあっても、必ずログインし直させる。
                    parameters.append("max_age", "0")
                }.buildString()
            return ReauthRequest(url, redirectUri, state, nonce, verifier)
        }
    }
}

/** 再認証の失敗。 */
class ReauthException(val reason: Reason, message: String) : Exception(message) {
    enum class Reason {
        /** AuthZ がエラーを返した、または利用者が取り消した。 */
        DENIED,

        /** 時間内に終わらなかった。 */
        TIMEOUT,

        /** ブラウザーを開けなかった。 */
        BROWSER_UNAVAILABLE,

        /** ID トークンを受け付けられない（iss、aud、exp、nonce の不一致など）。 */
        INVALID_ID_TOKEN,
    }
}

/** App Links で受け取ったリダイレクトの結果。 */
sealed interface ReauthCallback {
    data class Code(val code: String) : ReauthCallback

    data class Error(val error: String) : ReauthCallback
}

object ReauthCallbacks {
    /**
     * App Links で受け取った URI を読む。リダイレクト先のパスでないか、`state` が違えば `null`（受け取らずに
     * 待ち続ける。ほかのアプリが同じ URL でこのアプリを開いても、`state` を知らなければ何もできない）。
     */
    fun parse(uri: String, request: ReauthRequest): ReauthCallback? {
        val url =
            try {
                Url(uri)
            } catch (_: Exception) {
                return null
            }
        val expected = Url(request.redirectUri)
        if (url.protocol != expected.protocol || url.host != expected.host || url.port != expected.port) return null
        if (url.encodedPath.trimEnd('/') != expected.encodedPath.trimEnd('/')) return null
        if (url.parameters["state"] != request.state) return null
        url.parameters["error"]?.let { error ->
            val description = url.parameters["error_description"]?.let { " ($it)" }.orEmpty()
            return ReauthCallback.Error(error + description)
        }
        val code = url.parameters["code"]?.takeIf { it.isNotEmpty() } ?: return ReauthCallback.Error("認可コードがありません")
        return ReauthCallback.Code(code)
    }
}

/** ID トークンの `exp` の許容のずれ（Web、TUI と同じ 60 秒）。 */
private const val CLOCK_SKEW_SECONDS = 60L

/**
 * ID トークンの iss、aud、exp、nonce を確かめる（Web の `validateIdToken`、TUI の `validate_id_token` と同じ。
 * 署名はサーバーが AuthZ の JWKS で確かめる）。
 */
fun validateIdToken(idToken: String, issuer: String, clientId: String, nonce: String, nowSeconds: Long) {
    fun invalid(message: String): Nothing = throw ReauthException(ReauthException.Reason.INVALID_ID_TOKEN, message)
    val payload = idToken.split('.').getOrNull(1) ?: invalid("JWT の形式ではありません")
    val claims =
        try {
            CryptoStore.json.parseToJsonElement(
                Base64.getUrlDecoder().decode(payload.trimEnd('=')).decodeToString()
            ) as JsonObject
        } catch (_: Exception) {
            invalid("ペイロードを読めません")
        }
    fun text(key: String) = (claims[key] as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull
    if (text("iss")?.trimEnd('/') != issuer.trimEnd('/')) invalid("iss が一致しません")
    val audience =
        when (val aud = claims["aud"]) {
            is JsonPrimitive -> aud.isString && aud.content == clientId
            is JsonArray -> aud.any { (it as? JsonPrimitive)?.contentOrNull == clientId }
            else -> false
        }
    if (!audience) invalid("aud が一致しません")
    val exp = (claims["exp"] as? JsonPrimitive)?.longOrNull
    if (exp == null || exp <= nowSeconds - CLOCK_SKEW_SECONDS) invalid("期限切れです")
    if (text("nonce") != nonce) invalid("nonce が一致しません")
}
