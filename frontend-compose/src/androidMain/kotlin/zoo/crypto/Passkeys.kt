package zoo.crypto

import java.security.SecureRandom
import java.util.Base64
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject
import zoo.crypto.ffi.PasskeySlot

/**
 * パスキー（WebAuthn の PRF）で SSSS の鍵スロットを作り、開ける（docs/e2ee.md の「パスキーと Related Origin
 * Requests」。#360）。React の `passkey.ts` と同じ要求を、Android の Credential Manager の JSON の形で作る。
 *
 * - RP ID は AuthZ のホスト（ログインに使う issuer のホスト名。本番は `account.zli.works`）。Android アプリが
 *   この RP ID のパスキーを使うには、AuthZ の `/.well-known/assetlinks.json` でアプリを関連付ける（AuthZ#96）。
 * - アサーションはどこにも送らない。目的は PRF 出力だけ。
 */
enum class PasskeyError {
    /** 利用者が取り消した、または使えるパスキーがなかった。 */
    CANCELLED,

    /** 認証器か Credential Manager が PRF に対応していない（PRF の結果がない）。 */
    PRF_UNSUPPORTED,

    /** このアプリから RP ID のパスキーを使えない（assetlinks の関連付けがない）。 */
    ORIGIN_NOT_ALLOWED,

    /** 返ったパスキーに対応するスロットがない。 */
    NO_SLOT,

    UNKNOWN,
}

class PasskeyException(val error: PasskeyError, message: String, cause: Throwable? = null) : Exception(message, cause)

/** PRF の結果。`prfOutput`（32 バイト）は使い終わったら呼び出し側が 0 で埋める。 */
class PrfResult(
    /** base64url（パディングなし）の `rawId`。 */
    val credentialId: String,
    val prfOutput: ByteArray,
)

/** Credential Manager で PRF を評価する（アプリ側で実装する。単体テストでは差し替える）。 */
interface PasskeyClient {
    /** 鍵スロットを足すために、利用者が選んだパスキーで PRF を評価する（`allowCredentials` は空）。 */
    suspend fun prfForNewSlot(rpId: String, prfSalt: ByteArray): PrfResult

    /** 既存のパスキーのスロットで PRF を評価する（解錠）。 */
    suspend fun prfForUnlock(rpId: String, slots: List<PasskeySlot>): PrfResult
}

object PasskeyJson {
    private val random = SecureRandom()
    private val urlEncoder = Base64.getUrlEncoder().withoutPadding()

    fun base64Url(bytes: ByteArray): String = urlEncoder.encodeToString(bytes)

    /** パディングの有無を問わない base64url。 */
    fun decodeBase64Url(value: String): ByteArray = Base64.getUrlDecoder().decode(value.trimEnd('='))

    private fun challenge(): String = base64Url(ByteArray(32).also(random::nextBytes))

    /** 鍵スロットを足すための要求（`PublicKeyCredentialRequestOptionsJSON`）。 */
    fun newSlotRequest(rpId: String, prfSalt: ByteArray, challenge: String = challenge()): String =
        buildJsonObject {
            put("rpId", rpId)
            put("challenge", challenge)
            put("userVerification", "required")
            put("allowCredentials", JsonArray(emptyList()))
            putJsonObject("extensions") {
                putJsonObject("prf") { putJsonObject("eval") { put("first", base64Url(prfSalt)) } }
            }
        }.toString()

    /** 解錠の要求。すべてのパスキーのスロットを `allowCredentials` と `evalByCredential` に並べる。 */
    fun unlockRequest(rpId: String, slots: List<PasskeySlot>, challenge: String = challenge()): String =
        buildJsonObject {
            put("rpId", rpId)
            put("challenge", challenge)
            put("userVerification", "required")
            put(
                "allowCredentials",
                buildJsonArray {
                    slots.forEach { slot ->
                        add(
                            buildJsonObject {
                                put("type", "public-key")
                                put("id", slot.credentialId)
                            }
                        )
                    }
                },
            )
            putJsonObject("extensions") {
                putJsonObject("prf") {
                    putJsonObject("evalByCredential") {
                        slots.forEach { slot ->
                            putJsonObject(slot.credentialId) { put("first", base64Url(slot.prfSalt)) }
                        }
                    }
                }
            }
        }.toString()

    /**
     * Credential Manager の `authenticationResponseJson` から、`rawId` と PRF の結果を取り出す。
     * PRF の結果がない（または 32 バイトでない）ときは [PasskeyError.PRF_UNSUPPORTED]。
     */
    fun parseResponse(responseJson: String): PrfResult {
        val response =
            try {
                CryptoStore.json.parseToJsonElement(responseJson) as JsonObject
            } catch (failure: Exception) {
                throw PasskeyException(PasskeyError.UNKNOWN, "パスキーの応答を読めません", failure)
            }
        val rawId =
            ((response["rawId"] ?: response["id"]) as? JsonPrimitive)?.contentOrNull
                ?: throw PasskeyException(PasskeyError.UNKNOWN, "パスキーの応答に rawId がありません")
        // credential ID は、スロットの値と同じパディングなしの base64url にそろえる。
        val credentialId =
            try {
                base64Url(decodeBase64Url(rawId))
            } catch (failure: IllegalArgumentException) {
                throw PasskeyException(PasskeyError.UNKNOWN, "パスキーの rawId を読めません", failure)
            }
        val first =
            (((((response["clientExtensionResults"] as? JsonObject)?.get("prf") as? JsonObject)?.get("results")
                    as? JsonObject)
                    ?.get("first")) as? JsonPrimitive)
                ?.contentOrNull
        val output =
            first?.let {
                try {
                    decodeBase64Url(it)
                } catch (_: IllegalArgumentException) {
                    null
                }
            }
        if (output == null || output.size != 32) {
            output?.fill(0)
            throw PasskeyException(PasskeyError.PRF_UNSUPPORTED, "PRF の結果がありません")
        }
        return PrfResult(credentialId, output)
    }
}
