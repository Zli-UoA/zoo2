import AuthenticationServices
import CryptoKit
import Foundation
import UIKit

/// ログインと暗号化操作の再認証で共有する OIDC 認可コード要求。
/// PKCE の verifier、state、nonce はメモリーにだけ保持する。
/// AuthZ の iOS クライアントには `works.zli.zoo:/reauth-callback` を登録する。
struct ReauthRequest: Equatable {
    /// 認可リクエストの URL。
    let url: URL
    let redirectURI: String
    let state: String
    let nonce: String
    let verifier: String

    static let callbackScheme = "works.zli.zoo"
    static let redirectURI = "works.zli.zoo:/reauth-callback"
    /// 利用者がログインし直すのを待つ時間（Web、TUI、Android と同じ 10 分）。
    static let timeout: Duration = .seconds(10 * 60)

    /// 32 バイトの乱数の base64url（43 文字）。PKCE の verifier（43〜128 文字の unreserved）にも使える。
    static func randomToken(fill: (UnsafeMutableRawBufferPointer) -> OSStatus = {
        SecRandomCopyBytes(kSecRandomDefault, $0.count, $0.baseAddress!)
    }) throws -> String {
        var bytes = Data(count: 32)
        let status = bytes.withUnsafeMutableBytes(fill)
        guard status == errSecSuccess else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
        return PasskeyCoding.base64URL(bytes)
    }

    /// PKCE の S256 の code challenge。
    static func codeChallenge(_ verifier: String) -> String {
        PasskeyCoding.base64URL(Data(SHA256.hash(data: Data(verifier.utf8))))
    }

    /// 認可リクエストを作る。scope は `openid profile`（再認証では refresh token を作らないので `offline_access` を外す）。
    /// `resource` はふだんのログインと同じ値（空なら付けない）。
    static func create(authorizationEndpoint: String, clientID: String, resource: String,
                       state: String? = nil, nonce: String? = nil,
                       verifier: String? = nil, forceReauth: Bool = true,
                       scope: String = "openid profile offline_access") throws -> ReauthRequest {
        let state = try state ?? randomToken()
        let nonce = try nonce ?? randomToken()
        let verifier = try verifier ?? randomToken()
        guard var parts = URLComponents(string: authorizationEndpoint), parts.scheme == "https",
              parts.host != nil, parts.user == nil, parts.password == nil, parts.fragment == nil else {
            throw ZooError.invalidConfiguration
        }
        var items = [
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "client_id", value: clientID),
            URLQueryItem(name: "redirect_uri", value: redirectURI),
            URLQueryItem(name: "scope", value: forceReauth ? "openid profile" : scope)
        ]
        if !resource.trimmingCharacters(in: .whitespaces).isEmpty { items.append(URLQueryItem(name: "resource", value: resource)) }
        items += [
            URLQueryItem(name: "state", value: state),
            URLQueryItem(name: "nonce", value: nonce),
            URLQueryItem(name: "code_challenge", value: codeChallenge(verifier)),
            URLQueryItem(name: "code_challenge_method", value: "S256")
        ]
        // 通常ログインでは更新トークンの同意を取り、暗号鍵の保存直前だけ再認証を要求する。
        items.append(forceReauth ? URLQueryItem(name: "max_age", value: "0") : URLQueryItem(name: "prompt", value: "consent"))
        let existing = parts.queryItems ?? []
        let generatedNames = Set(items.map(\.name)).union(["max_age", "prompt"])
        guard Set(existing.map(\.name)).count == existing.count,
              existing.allSatisfy({ !generatedNames.contains($0.name) }) else { throw ZooError.invalidConfiguration }
        parts.queryItems = existing + items
        guard let url = parts.url else { throw ZooError.invalidConfiguration }
        return ReauthRequest(url: url, redirectURI: redirectURI, state: state, nonce: nonce, verifier: verifier)
    }
}

/// 再認証の失敗。
enum ReauthError: Error, Equatable, LocalizedError {
    /// AuthZ がエラーを返した、または利用者が取り消した。
    case denied(String)
    /// 時間内に終わらなかった。
    case timeout
    /// ブラウザーを開けなかった。
    case browserUnavailable
    /// ID トークンを受け付けられない（iss、aud、exp、nonce の不一致など）。
    case invalidIDToken(String)

    var errorDescription: String? {
        switch self {
        case .denied: "認証を完了できませんでした。もう一度お試しください。"
        case .timeout: "認証が時間内に終わりませんでした。もう一度お試しください。"
        case .browserUnavailable: "認証画面を開けませんでした。もう一度お試しください。"
        case .invalidIDToken: "認証サーバーの応答を確認できませんでした。もう一度ログインしてください。"
        }
    }
}

enum ReauthCallback: Equatable {
    case code(String)
    case error(String)

    /// リダイレクトで受け取った URL を読む。リダイレクト先でないか、`state` が違えば `nil`（受け取らない。
    /// ほかのアプリが同じ URL でこのアプリを開いても、`state` を知らなければ何もできない）。
    static func parse(_ url: URL, request: ReauthRequest) -> ReauthCallback? {
        guard var callback = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let items = callback.queryItems,
              Set(items.map(\.name)).count == items.count else { return nil }
        callback.query = nil
        guard callback.string == request.redirectURI else { return nil }
        func value(_ name: String) -> String? { items.first(where: { $0.name == name })?.value }
        guard value("state") == request.state,
              !(value("code") != nil && value("error") != nil) else { return nil }
        if let error = value("error") {
            return .error(error + (value("error_description").map { " (\($0))" } ?? ""))
        }
        guard let code = value("code"), !code.isEmpty else { return .error("認可コードがありません") }
        return .code(code)
    }
}

/// ID トークンの iss、aud、exp、nonce を確かめる（Web の `validateIdToken`、Android の `validateIdToken` と同じ）。
/// 署名はサーバーが AuthZ の JWKS で確かめる。`exp` の許容のずれは 60 秒。
func validateIDToken(_ idToken: String, issuer: String, clientID: String, nonce: String, now: Date = Date()) throws {
    func invalid(_ message: String) -> ReauthError { .invalidIDToken(message) }
    let parts = idToken.split(separator: ".", omittingEmptySubsequences: false)
    guard parts.count >= 2 else { throw invalid("JWT の形式ではありません") }
    guard let payload = PasskeyCoding.decodeBase64URL(String(parts[1])),
          let claims = try? JSONSerialization.jsonObject(with: payload) as? [String: Any] else {
        throw invalid("ペイロードを読めません")
    }
    guard let iss = claims["iss"] as? String, iss == issuer else { throw invalid("iss が一致しません") }
    let audience: Bool
    if let aud = claims["aud"] as? String { audience = aud == clientID }
    else if let aud = claims["aud"] as? [Any] { audience = aud.contains { ($0 as? String) == clientID } }
    else { audience = false }
    guard audience else { throw invalid("aud が一致しません") }
    guard let exp = (claims["exp"] as? NSNumber)?.doubleValue, exp > now.timeIntervalSince1970 - 60 else {
        throw invalid("期限切れです")
    }
    guard claims["nonce"] as? String == nonce else { throw invalid("nonce が一致しません") }
}

/// 直前の再認証で ID トークンを取る。返した ID トークンを `X-Zoo-Reauth` に入れる。
@MainActor
final class Reauthenticator: NSObject {
    private let auth: Authentication
    private var session: ASWebAuthenticationSession?
    private var pending: (id: UUID, continuation: CheckedContinuation<ReauthCallback, any Error>)?
    private var timeoutTask: Task<Void, Never>?

    init(auth: Authentication) { self.auth = auth }

    private var authenticating = false

    func idToken() async throws -> String {
        guard !authenticating else { throw ReauthError.denied("本人確認を実行中です。完了してから再試行してください。") }
        authenticating = true
        defer { authenticating = false }
        let discovery = try await auth.discovery()
        guard let endpoint = discovery.authorization_endpoint else {
            throw ZooError.auth("認証サーバーが再認証に対応していません。")
        }
        let request = try ReauthRequest.create(authorizationEndpoint: endpoint, clientID: auth.settings.clientID,
                                               resource: auth.settings.resource)
        let callback = try await open(request)
        let code: String
        switch callback {
        case .code(let value): code = value
        case .error(let reason): throw ReauthError.denied(reason)
        }
        let token = try await auth.exchangeReauthCode(code, verifier: request.verifier, redirectURI: request.redirectURI, discovery: discovery)
        try validateIDToken(token, issuer: discovery.issuer, clientID: auth.settings.clientID, nonce: request.nonce)
        return token
    }

    /// アプリ内の認証ブラウザーで、AuthZ のパスキー認証を開く。
    func open(_ request: ReauthRequest) async throws -> ReauthCallback {
        try Task.checkCancellation()
        guard pending == nil else { throw ReauthError.denied("認証を実行中です") }
        let id = UUID()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                guard !Task.isCancelled else { continuation.resume(throwing: CancellationError()); return }
                pending = (id, continuation)
                let browser = ASWebAuthenticationSession(url: request.url, callbackURLScheme: ReauthRequest.callbackScheme) { [weak self] url, error in
                    Task { @MainActor in
                        guard let self else { return }
                        if let error {
                            let cancelled = (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin
                            self.finish(.failure(cancelled ? CancellationError() : ReauthError.browserUnavailable), id: id)
                        } else if let url, let parsed = ReauthCallback.parse(url, request: request) {
                            self.finish(.success(parsed), id: id)
                        } else {
                            self.finish(.failure(ReauthError.denied("リダイレクトを読めません")), id: id)
                        }
                    }
                }
                browser.presentationContextProvider = self
                browser.prefersEphemeralWebBrowserSession = false
                session = browser
                timeoutTask = Task { @MainActor [weak self] in
                    do { try await Task.sleep(for: ReauthRequest.timeout) } catch { return }
                    self?.cancel(id: id, error: ReauthError.timeout)
                }
                if !browser.start() { finish(.failure(ReauthError.browserUnavailable), id: id) }
            }
        } onCancel: {
            Task { @MainActor in self.cancel(id: id, error: CancellationError()) }
        }
    }

    private func cancel(id: UUID, error: any Error) {
        guard pending?.id == id else { return }
        let browser = session
        // cancel の通知が来なくても待機を終え、遅れた通知を次の要求へ適用しない。
        finish(.failure(error), id: id)
        browser?.cancel()
    }

    private func finish(_ result: Result<ReauthCallback, any Error>, id: UUID) {
        guard let pending, pending.id == id else { return }
        self.pending = nil
        timeoutTask?.cancel()
        timeoutTask = nil
        session = nil
        pending.continuation.resume(with: result)
    }

}

extension Reauthenticator: ASWebAuthenticationPresentationContextProviding {
    nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        MainActor.assumeIsolated {
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows).first(where: \.isKeyWindow) ?? ASPresentationAnchor()
        }
    }
}
