import AuthenticationServices
import CryptoKit
import Foundation
import Security
import UIKit

struct TokenSet: Codable, Sendable {
    let access: String
    let refresh: String?
    let expires: Date
}

enum TokenVault {
    private static func query(_ account: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "works.zli.zoo.session",
         kSecAttrAccount as String: account]
    }

    static func load(_ account: String) -> TokenSet? {
        var request = query(account)
        request[kSecReturnData as String] = true
        var result: CFTypeRef?
        guard SecItemCopyMatching(request as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data else { return nil }
        return try? JSONDecoder().decode(TokenSet.self, from: data)
    }

    static func save(_ token: TokenSet, account: String) throws {
        let bytes = try JSONEncoder().encode(token)
        let attributes: [String: Any] = [kSecValueData as String: bytes,
                                        kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly]
        var status = SecItemUpdate(query(account) as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var request = query(account)
            request.merge(attributes) { _, new in new }
            status = SecItemAdd(request as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw ZooError.auth("ログイン情報を安全に保存できませんでした。") }
    }

    static func delete(_ account: String) { SecItemDelete(query(account) as CFDictionary) }
}

struct Discovery: Decodable, Sendable {
    let issuer: String
    let authorization_endpoint: String
    let token_endpoint: String
}

enum PKCE {
    static func random() throws -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
            throw ZooError.auth("認証の準備に失敗しました。")
        }
        return base64(Data(bytes))
    }

    static func base64(_ bytes: Data) -> String {
        bytes.base64EncodedString().replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }

    static func challenge(_ verifier: String) -> String { base64(Data(SHA256.hash(data: Data(verifier.utf8)))) }

    static func code(from callback: URL, expectedState: String, expectedIssuer: String? = nil) throws -> String {
        guard callback.scheme == "https", callback.host == "zli.chat", callback.path == "/ios/oauth/callback",
              callback.port == nil, callback.user == nil, callback.password == nil, callback.fragment == nil,
              let query = URLComponents(url: callback, resolvingAgainstBaseURL: false)?.queryItems else {
            throw ZooError.auth("認証の戻り先を確認できませんでした。")
        }
        let states = query.filter { $0.name == "state" }
        let codes = query.filter { $0.name == "code" }
        guard states.count == 1, states.first?.value == expectedState,
              !query.contains(where: { $0.name == "error" }), codes.count == 1,
              let code = codes.first?.value, !code.isEmpty else {
            throw ZooError.auth("認証を完了できませんでした。もう一度ログインしてください。")
        }
        let issuers = query.filter { $0.name == "iss" }
        if let expectedIssuer, (issuers.count != 1 || issuers.first?.value != expectedIssuer) {
            throw ZooError.auth("認証サーバーの識別情報が一致しません。")
        }
        return code
    }

    static func form(_ fields: [String: String]) -> Data {
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")
        return Data(fields.sorted { $0.key < $1.key }.map {
            "\($0.key.addingPercentEncoding(withAllowedCharacters: allowed)!)=\($0.value.addingPercentEncoding(withAllowedCharacters: allowed)!)"
        }.joined(separator: "&").utf8)
    }
}

@MainActor
final class Authentication: NSObject, ASWebAuthenticationPresentationContextProviding {
    let settings: ConnectionSettings
    let transport: HTTPTransport
    private var tokens: TokenSet?
    private var browser: ASWebAuthenticationSession?
    private var refreshTask: Task<TokenSet, Error>?
    private var generation = UUID()

    init(settings: ConnectionSettings, transport: HTTPTransport = HTTPTransport()) {
        self.settings = settings
        self.transport = transport
        self.tokens = TokenVault.load(settings.storageKey)
    }

    var hasSession: Bool { tokens != nil }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first(where: \.isKeyWindow) ?? ASPresentationAnchor()
    }

    func discovery() async throws -> Discovery {
        let issuer = try ConnectionSettings.secureURL(settings.issuer)
        let (data, response) = try await transport.send(URLRequest(url: issuer.appendingPathComponent(".well-known/openid-configuration")))
        try HTTPTransport.check(data, response)
        let discovery = try JSONDecoder().decode(Discovery.self, from: data)
        guard discovery.issuer.trimmingCharacters(in: CharacterSet(charactersIn: "/")) ==
                settings.issuer.trimmingCharacters(in: CharacterSet(charactersIn: "/")) else {
            throw ZooError.auth("認証サーバーの識別情報が一致しません。")
        }
        _ = try ConnectionSettings.secureURL(discovery.authorization_endpoint)
        _ = try ConnectionSettings.secureURL(discovery.token_endpoint)
        return discovery
    }

    func login() async throws {
        try settings.validate()
        let currentGeneration = generation
        let discovery = try await discovery()
        let verifier = try PKCE.random()
        let state = try PKCE.random()
        var url = URLComponents(string: discovery.authorization_endpoint)!
        var fields = ["client_id": settings.clientID, "redirect_uri": ConnectionSettings.callback,
                      "response_type": "code", "scope": settings.scope, "state": state,
                      "code_challenge": PKCE.challenge(verifier), "code_challenge_method": "S256"]
        if !settings.resource.isEmpty { fields["resource"] = settings.resource }
        url.queryItems = fields.map { URLQueryItem(name: $0.key, value: $0.value) }
        guard let authorizationURL = url.url else { throw ZooError.invalidConfiguration }
        let callback: URL = try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(url: authorizationURL, callback: .https(host: "zli.chat", path: "/ios/oauth/callback")) { url, error in
                if let url { continuation.resume(returning: url) }
                else { continuation.resume(throwing: error ?? ZooError.auth("ログインを中止しました。")) }
            }
            session.presentationContextProvider = self
            session.prefersEphemeralWebBrowserSession = true
            browser = session
            if !session.start() { continuation.resume(throwing: ZooError.auth("ログイン画面を開けませんでした。")) }
        }
        browser = nil
        let code = try PKCE.code(from: callback, expectedState: state, expectedIssuer: discovery.issuer)
        var exchange = ["grant_type": "authorization_code", "client_id": settings.clientID,
                        "redirect_uri": ConnectionSettings.callback, "code": code, "code_verifier": verifier]
        if !settings.resource.isEmpty { exchange["resource"] = settings.resource }
        let token = try await exchangeToken(endpoint: discovery.token_endpoint, fields: exchange, previousRefresh: nil)
        guard currentGeneration == generation else { throw CancellationError() }
        try TokenVault.save(token, account: settings.storageKey)
        tokens = token
    }

    private func exchangeToken(endpoint: String, fields: [String: String], previousRefresh: String?) async throws -> TokenSet {
        var request = URLRequest(url: try ConnectionSettings.secureURL(endpoint))
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = PKCE.form(fields)
        let (data, response) = try await transport.send(request)
        try HTTPTransport.check(data, response)
        struct Response: Decodable {
            let access_token: String
            let refresh_token: String?
            let expires_in: Double
            let token_type: String
        }
        let decoded = try JSONDecoder().decode(Response.self, from: data)
        guard !decoded.access_token.isEmpty, decoded.token_type.lowercased() == "bearer", decoded.expires_in > 0 else {
            throw ZooError.invalidResponse
        }
        return TokenSet(access: decoded.access_token, refresh: decoded.refresh_token ?? previousRefresh,
                        expires: Date().addingTimeInterval(decoded.expires_in))
    }

    func accessToken() async throws -> String {
        guard let tokens else { throw ZooError.expiredSession }
        if tokens.expires.timeIntervalSinceNow > 30 { return tokens.access }
        if let refreshTask {
            let epoch = generation
            do {
                let token = try await refreshTask.value
                guard epoch == generation else { throw CancellationError() }
                return token.access
            } catch ZooError.rejected(let status, _) where status == 400 || status == 401 {
                throw ZooError.expiredSession
            }
        }
        guard let refresh = tokens.refresh else { throw ZooError.expiredSession }
        let currentGeneration = generation
        let task = Task {
            let discovery = try await self.discovery()
            var fields = ["grant_type": "refresh_token", "client_id": settings.clientID, "refresh_token": refresh]
            if !settings.resource.isEmpty { fields["resource"] = settings.resource }
            return try await self.exchangeToken(endpoint: discovery.token_endpoint, fields: fields, previousRefresh: refresh)
        }
        refreshTask = task
        defer { if currentGeneration == generation { refreshTask = nil } }
        do {
            let fresh = try await task.value
            guard currentGeneration == generation else { throw CancellationError() }
            try TokenVault.save(fresh, account: settings.storageKey)
            self.tokens = fresh
            return fresh.access
        } catch ZooError.rejected(let status, _) where status == 400 || status == 401 {
            throw ZooError.expiredSession
        }
    }

    func logout() {
        generation = UUID()
        browser?.cancel()
        browser = nil
        refreshTask?.cancel()
        refreshTask = nil
        tokens = nil
        TokenVault.delete(settings.storageKey)
    }
}
