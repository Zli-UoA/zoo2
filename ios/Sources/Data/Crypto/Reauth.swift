import AuthenticationServices
import CryptoKit
import Foundation
import UIKit

/// E2EE の直前の再認証（docs/e2ee.md の「初回設定と解錠の流れ」の 7 と「クロス署名」の API。#685）。
///
/// iOS はふだんのログインにデバイス認可グラントを使うので、`max_age` を付けた ID トークンを取れない。
/// 再認証のときだけ、認可コードフロー（PKCE の S256、`max_age=0`、`state`、`nonce`）を `ASWebAuthenticationSession`
/// で開き、`works.zli.zoo:/reauth-callback` で認可コードを受け取って、トークンエンドポイントで ID トークンに換える
/// （Android の `Reauth.kt`、TUI の `auth/reauth.rs` と同じ要求と検査）。今のセッションのトークンは変えない。
/// PKCE の code verifier と `nonce` はメモリーにだけ置く。
///
/// AuthZ の iOS のクライアントには、リダイレクト URI `works.zli.zoo:/reauth-callback` を登録し、zoo2-infra の
/// `OIDC_REAUTH_AUDIENCES` にそのクライアント ID を足す。
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
                       verifier: String? = nil) throws -> ReauthRequest {
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
            URLQueryItem(name: "scope", value: "openid profile")
        ]
        if !resource.trimmingCharacters(in: .whitespaces).isEmpty { items.append(URLQueryItem(name: "resource", value: resource)) }
        items += [
            URLQueryItem(name: "state", value: state),
            URLQueryItem(name: "nonce", value: nonce),
            URLQueryItem(name: "code_challenge", value: codeChallenge(verifier)),
            URLQueryItem(name: "code_challenge_method", value: "S256"),
            // 既存の AuthZ のセッションがあっても、必ずログインし直させる。
            URLQueryItem(name: "max_age", value: "0")
        ]
        let existing = parts.queryItems ?? []
        let generatedNames = Set(items.map(\.name))
        guard Set(existing.map(\.name)).count == existing.count,
              existing.allSatisfy({ !generatedNames.contains($0.name) }) else { throw ZooError.invalidConfiguration }
        parts.queryItems = existing + items
        guard let url = parts.url else { throw ZooError.invalidConfiguration }
        return ReauthRequest(url: url, redirectURI: redirectURI, state: state, nonce: nonce, verifier: verifier)
    }
}

/// 再認証の失敗。
enum ReauthError: Error, Equatable {
    /// AuthZ がエラーを返した、または利用者が取り消した。
    case denied(String)
    /// 時間内に終わらなかった。
    case timeout
    /// ブラウザーを開けなかった。
    case browserUnavailable
    /// ID トークンを受け付けられない（iss、aud、exp、nonce の不一致など）。
    case invalidIDToken(String)
}

enum ReauthCallback: Equatable {
    case code(String)
    case error(String)

    /// リダイレクトで受け取った URL を読む。リダイレクト先でないか、`state` が違えば `nil`（受け取らない。
    /// ほかのアプリが同じ URL でこのアプリを開いても、`state` を知らなければ何もできない）。
    static func parse(_ url: URL, request: ReauthRequest) -> ReauthCallback? {
        guard let expected = URL(string: request.redirectURI),
              url.scheme == expected.scheme, url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
                == expected.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")),
              let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems else { return nil }
        func value(_ name: String) -> String? { items.first(where: { $0.name == name })?.value }
        guard value("state") == request.state else { return nil }
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
    private var timedOut = false

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
        let token = try await auth.exchangeReauthCode(code, verifier: request.verifier, redirectURI: request.redirectURI)
        try validateIDToken(token, issuer: discovery.issuer, clientID: auth.settings.clientID, nonce: request.nonce)
        return token
    }

    private func open(_ request: ReauthRequest) async throws -> ReauthCallback {
        timedOut = false
        // 時間内に終わらなければ、ブラウザーを閉じて `timeout` にする。
        let timer = Task { @MainActor in
            try await Task.sleep(for: ReauthRequest.timeout)
            timedOut = true
            session?.cancel()
        }
        defer { timer.cancel(); session = nil }
        do {
            return try await withCheckedThrowingContinuation { continuation in
                let session = ASWebAuthenticationSession(url: request.url, callbackURLScheme: ReauthRequest.callbackScheme) { url, error in
                    if let error {
                        let cancelled = (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin
                        continuation.resume(throwing: cancelled ? ReauthError.denied("取り消しました") : ReauthError.browserUnavailable)
                    } else if let url, let parsed = ReauthCallback.parse(url, request: request) {
                        continuation.resume(returning: parsed)
                    } else {
                        continuation.resume(throwing: ReauthError.denied("リダイレクトを読めません"))
                    }
                }
                session.presentationContextProvider = self
                // 既存の AuthZ のセッションがあっても、max_age=0 でログインし直させる。
                session.prefersEphemeralWebBrowserSession = false
                self.session = session
                if !session.start() { continuation.resume(throwing: ReauthError.browserUnavailable) }
            }
        } catch ReauthError.denied where timedOut {
            throw ReauthError.timeout
        }
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
