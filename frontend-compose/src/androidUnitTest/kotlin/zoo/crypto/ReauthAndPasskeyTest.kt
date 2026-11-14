package zoo.crypto

import io.ktor.http.Url
import java.util.Base64
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject
import zoo.crypto.ffi.PasskeySlot

/** 再認証（Custom Tabs と App Links）とパスキー（Credential Manager の JSON）の補助（#360）。 */
class ReauthAndPasskeyTest {
    private val authorize = "https://account.zli.works/api/auth/oauth2/authorize"

    private fun request() =
        ReauthRequest.create(
            authorize,
            clientId = "zoo-android",
            redirectUri = ReauthRequest.redirectUri("https://zli.chat/"),
            resource = "https://zli.chat/api",
            state = "state-1",
            nonce = "nonce-1",
            verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        )

    @Test
    fun buildsTheAuthorizationRequest() {
        val request = request()
        assertEquals("https://zli.chat/android/reauth-callback", request.redirectUri)
        val url = Url(request.url)
        assertEquals("account.zli.works", url.host)
        assertEquals("code", url.parameters["response_type"])
        assertEquals("zoo-android", url.parameters["client_id"])
        assertEquals("https://zli.chat/android/reauth-callback", url.parameters["redirect_uri"])
        // 再認証では refresh token を作らない。
        assertEquals("openid profile", url.parameters["scope"])
        assertEquals("https://zli.chat/api", url.parameters["resource"])
        assertEquals("state-1", url.parameters["state"])
        assertEquals("nonce-1", url.parameters["nonce"])
        // RFC 7636 の付録 B の例。
        assertEquals("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM", url.parameters["code_challenge"])
        assertEquals("S256", url.parameters["code_challenge_method"])
        assertEquals("0", url.parameters["max_age"])
        // verifier と nonce は文字列表現に出さない。
        assertTrue("dBjft" !in request.toString())
        assertFailsWith<IllegalArgumentException> {
            ReauthRequest.create("http://account.zli.works/authorize", "c", request.redirectUri, "")
        }
        val tokens = setOf(ReauthRequest.randomToken(), ReauthRequest.randomToken())
        assertEquals(2, tokens.size)
        assertTrue(tokens.all { it.length == 43 })
    }

    @Test
    fun acceptsOnlyTheMatchingCallback() {
        val request = request()
        val base = "https://zli.chat/android/reauth-callback"
        assertEquals(ReauthCallback.Code("abc"), ReauthCallbacks.parse("$base?code=abc&state=state-1", request))
        assertEquals(
            ReauthCallback.Error("access_denied (cancelled)"),
            ReauthCallbacks.parse("$base?error=access_denied&error_description=cancelled&state=state-1", request),
        )
        assertEquals(ReauthCallback.Error("認可コードがありません"), ReauthCallbacks.parse("$base?state=state-1", request))
        // state が違う、別のパスや別のホストのものは受け取らない（待ち続ける）。
        assertNull(ReauthCallbacks.parse("$base?code=abc&state=other", request))
        assertNull(ReauthCallbacks.parse("$base?code=abc", request))
        assertNull(ReauthCallbacks.parse("https://zli.chat/android/other?code=abc&state=state-1", request))
        assertNull(ReauthCallbacks.parse("https://evil.example/android/reauth-callback?code=abc&state=state-1", request))
        assertNull(ReauthCallbacks.parse("http://zli.chat/android/reauth-callback?code=abc&state=state-1", request))
        assertNull(ReauthCallbacks.parse("not a url", request))
    }

    private fun idToken(claims: Map<String, Any>): String {
        val payload =
            buildJsonObject {
                claims.forEach { (key, value) ->
                    when (value) {
                        is String -> put(key, value)
                        is Long -> put(key, value)
                        is List<*> -> put(key, JsonArray(value.map { JsonPrimitive(it as String) }))
                    }
                }
            }
        val encode = { text: String -> Base64.getUrlEncoder().withoutPadding().encodeToString(text.encodeToByteArray()) }
        return "${encode("{\"alg\":\"RS256\"}")}.${encode(payload.toString())}.signature"
    }

    @Test
    fun validatesTheIdToken() {
        val issuer = "https://account.zli.works/api/auth"
        val now = 1_800_000_000L
        val good = mapOf("iss" to "$issuer/", "aud" to "zoo-android", "exp" to now + 300, "nonce" to "n")
        validateIdToken(idToken(good), issuer, "zoo-android", "n", now)
        validateIdToken(idToken(good + ("aud" to listOf("other", "zoo-android"))), issuer, "zoo-android", "n", now)
        // 時計のずれは 60 秒まで認める。
        validateIdToken(idToken(good + ("exp" to now - 30)), issuer, "zoo-android", "n", now)
        for (bad in listOf(
            good + ("iss" to "https://evil.example"),
            good + ("aud" to "other"),
            good + ("aud" to listOf("other")),
            good + ("exp" to now - 61),
            good - "exp",
            good + ("nonce" to "other"),
            good - "nonce",
        )) {
            val failure =
                assertFailsWith<ReauthException> { validateIdToken(idToken(bad), issuer, "zoo-android", "n", now) }
            assertEquals(ReauthException.Reason.INVALID_ID_TOKEN, failure.reason)
        }
        assertFailsWith<ReauthException> { validateIdToken("not-a-jwt", issuer, "zoo-android", "n", now) }
        assertFailsWith<ReauthException> { validateIdToken("a.@@@.c", issuer, "zoo-android", "n", now) }
    }

    @Test
    fun buildsCredentialManagerRequests() {
        val salt = ByteArray(32) { it.toByte() }
        val newSlot = Json.parseToJsonElement(PasskeyJson.newSlotRequest("account.zli.works", salt, "Y2hhbGxlbmdl")).jsonObject
        assertEquals("account.zli.works", newSlot["rpId"]!!.jsonPrimitive.content)
        assertEquals("required", newSlot["userVerification"]!!.jsonPrimitive.content)
        assertEquals(0, newSlot["allowCredentials"]!!.jsonArray.size)
        assertEquals(
            PasskeyJson.base64Url(salt),
            newSlot["extensions"]!!.jsonObject["prf"]!!.jsonObject["eval"]!!.jsonObject["first"]!!.jsonPrimitive.content,
        )

        val slots =
            listOf(
                PasskeySlot("s1", "Y3JlZDE", ByteArray(32) { 1 }, null),
                PasskeySlot("s2", "Y3JlZDI", ByteArray(32) { 2 }, "Pixel"),
            )
        val unlock = Json.parseToJsonElement(PasskeyJson.unlockRequest("account.zli.works", slots, "Y2hhbGxlbmdl")).jsonObject
        assertEquals(
            listOf("Y3JlZDE", "Y3JlZDI"),
            unlock["allowCredentials"]!!.jsonArray.map { it.jsonObject["id"]!!.jsonPrimitive.content },
        )
        val byCredential = unlock["extensions"]!!.jsonObject["prf"]!!.jsonObject["evalByCredential"]!!.jsonObject
        assertEquals(PasskeyJson.base64Url(ByteArray(32) { 2 }), byCredential["Y3JlZDI"]!!.jsonObject["first"]!!.jsonPrimitive.content)
    }

    @Test
    fun parsesCredentialManagerResponses() {
        val output = ByteArray(32) { 7 }
        fun response(first: String?, rawId: String = "Y3JlZDE=") =
            buildJsonObject {
                put("id", "Y3JlZDE")
                put("rawId", rawId)
                put("type", "public-key")
                putJsonObject("clientExtensionResults") {
                    if (first != null) putJsonObject("prf") { putJsonObject("results") { put("first", first) } }
                }
            }.toString()
        val result = PasskeyJson.parseResponse(response(PasskeyJson.base64Url(output)))
        // スロットの credential ID と同じ、パディングなしの base64url にそろえる。
        assertEquals("Y3JlZDE", result.credentialId)
        assertContentEquals(output, result.prfOutput)

        for (missing in listOf(response(null), response(PasskeyJson.base64Url(ByteArray(16))), response("@@"))) {
            assertEquals(PasskeyError.PRF_UNSUPPORTED, assertFailsWith<PasskeyException> { PasskeyJson.parseResponse(missing) }.error)
        }
        assertEquals(PasskeyError.UNKNOWN, assertFailsWith<PasskeyException> { PasskeyJson.parseResponse("{}") }.error)
        assertEquals(PasskeyError.UNKNOWN, assertFailsWith<PasskeyException> { PasskeyJson.parseResponse("[") }.error)
    }
}
