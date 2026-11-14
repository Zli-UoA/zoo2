package zoo.android

import android.app.Activity
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.NoCredentialException
import androidx.credentials.exceptions.publickeycredential.GetPublicKeyCredentialDomException
import zoo.crypto.PasskeyClient
import zoo.crypto.PasskeyError
import zoo.crypto.PasskeyException
import zoo.crypto.PasskeyJson
import zoo.crypto.PrfResult
import zoo.crypto.ffi.PasskeySlot

/**
 * Credential Manager のパスキーで PRF を評価する（docs/e2ee.md の「パスキーと Related Origin Requests」。#360）。
 *
 * RP ID（AuthZ のホスト）はアプリのパッケージと別のドメインなので、AuthZ が `/.well-known/assetlinks.json` で
 * このアプリを関連付けていないと使えない（AuthZ#96）。【未検証】Credential Manager の要求 JSON の `prf` 拡張で
 * PRF の結果を得られるかは、実機で確かめる（docs/e2ee.md の「使えない環境」）。結果がなければ
 * [PasskeyError.PRF_UNSUPPORTED] にして、リカバリーキーを案内する。
 */
internal class AndroidPasskeys(private val activity: Activity) : PasskeyClient {
    private val manager = CredentialManager.create(activity)

    override suspend fun prfForNewSlot(rpId: String, prfSalt: ByteArray): PrfResult =
        evaluate(PasskeyJson.newSlotRequest(rpId, prfSalt))

    override suspend fun prfForUnlock(rpId: String, slots: List<PasskeySlot>): PrfResult =
        evaluate(PasskeyJson.unlockRequest(rpId, slots))

    private suspend fun evaluate(requestJson: String): PrfResult {
        val response =
            try {
                manager.getCredential(activity, GetCredentialRequest(listOf(GetPublicKeyCredentialOption(requestJson))))
            } catch (failure: GetCredentialCancellationException) {
                throw PasskeyException(PasskeyError.CANCELLED, "パスキーの操作が取り消されました", failure)
            } catch (failure: NoCredentialException) {
                throw PasskeyException(PasskeyError.CANCELLED, "使えるパスキーがありません", failure)
            } catch (failure: GetPublicKeyCredentialDomException) {
                val error =
                    when (failure.domError::class.simpleName) {
                        "NotAllowedError", "AbortError" -> PasskeyError.CANCELLED
                        "SecurityError" -> PasskeyError.ORIGIN_NOT_ALLOWED
                        "NotSupportedError" -> PasskeyError.PRF_UNSUPPORTED
                        else -> PasskeyError.UNKNOWN
                    }
                throw PasskeyException(error, "パスキーで解錠できません", failure)
            } catch (failure: GetCredentialException) {
                throw PasskeyException(PasskeyError.UNKNOWN, "パスキーで解錠できません", failure)
            }
        val credential =
            response.credential as? PublicKeyCredential
                ?: throw PasskeyException(PasskeyError.CANCELLED, "パスキーが選ばれませんでした")
        return PasskeyJson.parseResponse(credential.authenticationResponseJson)
    }
}
