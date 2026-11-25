import AuthenticationServices
import Foundation
import UIKit
import ZooCrypto

/// パスキー（WebAuthn の PRF）で SSSS の鍵スロットを作り、開ける（docs/e2ee.md の「パスキーと Related Origin
/// Requests」。#360）。Web の `passkey.ts`、Android の `Passkeys.kt` と同じ要求を、`ASAuthorization` で作る。
///
/// - RP ID は AuthZ のホスト（ログインに使う issuer のホスト名。本番は `account.zli.works`）。アプリがこの RP ID の
///   パスキーを使うには、Associated Domains の `webcredentials:<RP ID>` と、AuthZ の
///   `/.well-known/apple-app-site-association` でアプリを関連付ける。
/// - アサーションはどこにも送らない。目的は PRF 出力だけ。
/// - PRF は iOS 18 以上。
enum PasskeyCoding {
    /// パディングなしの base64url。
    static func base64URL(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    /// パディングの有無を問わない base64url。読めなければ `nil`。
    static func decodeBase64URL(_ value: String) -> Data? {
        var text = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
            .trimmingCharacters(in: CharacterSet(charactersIn: "="))
        text += String(repeating: "=", count: (4 - text.count % 4) % 4)
        return Data(base64Encoded: text)
    }

    /// 32 バイトの PRF の出力でなければ `prfUnsupported`。
    static func checkedOutput(_ output: Data?) throws -> Data {
        guard let output, output.count == 32 else { throw PasskeyError.prfUnsupported }
        return output
    }

    /// `ASAuthorizationError` を、`PasskeyError` にする。Associated Domains の関連付けがないときは
    /// `.failed`（1004）で、説明に「not associated with domain」が入る。
    static func map(_ error: any Error) -> PasskeyError {
        guard let error = error as? ASAuthorizationError else { return .unknown }
        if error.code == .canceled { return .cancelled }
        if error.code == .failed, error.localizedDescription.contains("not associated with domain") { return .originNotAllowed }
        return .unknown
    }
}

@MainActor
final class SystemPasskeyClient: NSObject, PasskeyClient {
    private var continuation: CheckedContinuation<ASAuthorization, any Error>?
    private var controller: ASAuthorizationController?

    func prfForNewSlot(rpID: String, salt: Data) async throws -> PasskeyResult {
        // 利用者が選んだパスキーで PRF を評価する（`allowCredentials` は空）。
        try await assert(rpID: rpID, allowed: [], input: .inputValues(.saltInput1(salt)))
    }

    func prfForUnlock(rpID: String, slots: [PasskeySlot]) async throws -> PasskeyResult {
        var perCredential: [Data: ASAuthorizationPublicKeyCredentialPRFAssertionInput.InputValues] = [:]
        var allowed: [Data] = []
        for slot in slots {
            guard let id = PasskeyCoding.decodeBase64URL(slot.credentialId) else { continue }
            allowed.append(id)
            perCredential[id] = .saltInput1(slot.prfSalt)
        }
        return try await assert(rpID: rpID, allowed: allowed, input: .perCredentialInputValues(perCredential))
    }

    private func assert(rpID: String, allowed: [Data], input: ASAuthorizationPublicKeyCredentialPRFAssertionInput) async throws -> PasskeyResult {
        guard #available(iOS 18.0, *) else { throw PasskeyError.prfUnsupported }
        var challenge = Data(count: 32)
        let status = challenge.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 32, $0.baseAddress!) }
        guard status == errSecSuccess else { throw PasskeyError.unknown }
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpID)
        let request = provider.createCredentialAssertionRequest(challenge: challenge)
        request.userVerificationPreference = .required
        request.allowedCredentials = allowed.map { ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0) }
        request.prf = input
        let authorization: ASAuthorization
        do { authorization = try await perform(request) } catch { throw PasskeyCoding.map(error) }
        guard let assertion = authorization.credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion else {
            throw PasskeyError.unknown
        }
        let output = assertion.prf?.first.withUnsafeBytes { Data($0) }
        // credential ID は、スロットの値と同じパディングなしの base64url にそろえる。
        return PasskeyResult(credentialID: PasskeyCoding.base64URL(assertion.credentialID),
                             prfOutput: try PasskeyCoding.checkedOutput(output))
    }

    /// 保留中のパスキーの要求を取り消す（シートを閉じたとき、新しい要求を始めるとき）。
    func cancel() {
        controller?.cancel()
        finish(.failure(ASAuthorizationError(.canceled)))
    }

    private func finish(_ result: Result<ASAuthorization, any Error>) {
        continuation?.resume(with: result)
        continuation = nil
        controller = nil
    }

    private func perform(_ request: ASAuthorizationRequest) async throws -> ASAuthorization {
        // 前の要求が応答しないまま残っていても、新しい要求は始められる（前のものは取り消す）。
        cancel()
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            let controller = ASAuthorizationController(authorizationRequests: [request])
            controller.delegate = self
            controller.presentationContextProvider = self
            self.controller = controller
            controller.performRequests()
        }
    }
}

extension SystemPasskeyClient: ASAuthorizationControllerDelegate {
    nonisolated func authorizationController(controller: ASAuthorizationController,
                                             didCompleteWithAuthorization authorization: ASAuthorization) {
        Task { @MainActor in self.finish(.success(authorization)) }
    }

    nonisolated func authorizationController(controller: ASAuthorizationController, didCompleteWithError error: any Error) {
        Task { @MainActor in self.finish(.failure(error)) }
    }
}

extension SystemPasskeyClient: ASAuthorizationControllerPresentationContextProviding {
    nonisolated func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        MainActor.assumeIsolated {
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows).first(where: \.isKeyWindow) ?? ASPresentationAnchor()
        }
    }
}
