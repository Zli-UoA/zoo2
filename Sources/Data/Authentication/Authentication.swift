import Foundation
import Security

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
    let token_endpoint: String
    let device_authorization_endpoint: String?
}

enum OAuthForm {
    static func form(_ fields: [String: String]) -> Data {
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")
        return Data(fields.sorted { $0.key < $1.key }.map {
            "\($0.key.addingPercentEncoding(withAllowedCharacters: allowed)!)=\($0.value.addingPercentEncoding(withAllowedCharacters: allowed)!)"
        }.joined(separator: "&").utf8)
    }
}

@MainActor
final class Authentication {
    let settings: ConnectionSettings
    let transport: HTTPTransport
    private var tokens: TokenSet?
    private var refreshTask: Task<TokenSet, Error>?
    private var generation = UUID()

    init(settings: ConnectionSettings, transport: HTTPTransport = HTTPTransport()) {
        self.settings = settings
        self.transport = transport
        self.tokens = TokenVault.load(settings.storageKey)
    }

    var hasSession: Bool { tokens != nil }

    func discovery() async throws -> Discovery {
        let issuer = try ConnectionSettings.secureURL(settings.issuer)
        let (data, response) = try await transport.send(URLRequest(url: issuer.appendingPathComponent(".well-known/openid-configuration")))
        try HTTPTransport.check(data, response)
        let discovery = try JSONDecoder().decode(Discovery.self, from: data)
        guard discovery.issuer.trimmingCharacters(in: CharacterSet(charactersIn: "/")) ==
                settings.issuer.trimmingCharacters(in: CharacterSet(charactersIn: "/")) else {
            throw ZooError.auth("認証サーバーの識別情報が一致しません。")
        }
        _ = try ConnectionSettings.secureURL(discovery.token_endpoint)
        return discovery
    }

    func login(clock: some Clock<Duration> = ContinuousClock(),
               show: (DeviceAuthorizationPrompt) -> Void) async throws {
        try Task.checkCancellation()
        try settings.validate()
        // 手動ログインの開始後に、以前の更新処理が保存情報を上書きしないようにする。
        logout()
        let epoch = generation
        let discovery = try await discovery()
        try checkLogin(epoch)
        guard let endpoint = discovery.device_authorization_endpoint else {
            throw ZooError.auth("認証サーバーがデバイスコード認証に対応していません。")
        }
        var fields = ["client_id": settings.clientID, "scope": settings.scope]
        if !settings.resource.isEmpty { fields["resource"] = settings.resource }
        let started = clock.now
        let (data, response) = try await postForm(endpoint: endpoint, fields: fields)
        try checkLogin(epoch)
        guard (200..<300).contains(response.statusCode) else {
            throw deviceLoginError(data, status: response.statusCode)
        }
        let grant = try JSONDecoder().decode(DeviceAuthorizationResponse.self, from: data)
        let prompt = try grant.prompt(issuer: settings.issuer)
        let deadline = started.advanced(by: .seconds(grant.expires_in))
        guard clock.now < deadline else { throw ZooError.auth("確認コードの期限が切れました。ログインし直してください。") }
        show(prompt)
        var interval = Duration.seconds(grant.interval ?? 5)
        var tokenFields = ["grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                           "client_id": settings.clientID, "device_code": grant.device_code]
        if !settings.resource.isEmpty { tokenFields["resource"] = settings.resource }
        while true {
            try checkLogin(epoch)
            try await clock.sleep(until: min(clock.now.advanced(by: interval), deadline), tolerance: nil)
            try checkLogin(epoch)
            guard clock.now < deadline else {
                throw ZooError.auth("確認コードの期限が切れました。ログインし直してください。")
            }
            let remaining = clock.now.duration(to: deadline).components
            let remainingSeconds = Double(remaining.seconds) + Double(remaining.attoseconds) / 1e18
            let timeout = min(30, remainingSeconds)
            let result: (Data, HTTPURLResponse)
            do {
                result = try await postForm(endpoint: discovery.token_endpoint, fields: tokenFields, timeout: timeout)
            } catch let error as URLError where error.code == .timedOut {
                // RFC 8628 に従い、タイムアウトが続く間は問い合わせ間隔を広げる。
                interval *= 2
                continue
            }
            try checkLogin(epoch)
            guard clock.now < deadline else {
                throw ZooError.auth("確認コードの期限が切れました。ログインし直してください。")
            }
            let (tokenData, tokenResponse) = result
            if tokenResponse.statusCode == 429 {
                // 承認待ち中のレート制限でも、サーバーの待機指定とコードの期限を守る。
                interval = max(interval * 2, interval + .seconds(5))
                if let retryAfter = tokenResponse.value(forHTTPHeaderField: "Retry-After") {
                    let seconds: TimeInterval?
                    if let delta = TimeInterval(retryAfter), delta.isFinite, delta >= 0 {
                        seconds = delta
                    } else {
                        let formatter = DateFormatter()
                        formatter.locale = Locale(identifier: "en_US_POSIX")
                        formatter.timeZone = TimeZone(secondsFromGMT: 0)
                        formatter.dateFormat = "EEE, dd MMM yyyy HH:mm:ss zzz"
                        seconds = formatter.date(from: retryAfter)?.timeIntervalSinceNow
                    }
                    if let seconds, seconds.isFinite, seconds > 0 {
                        interval = max(interval, .seconds(min(seconds, remainingSeconds)))
                    }
                }
                continue
            }
            if tokenResponse.statusCode == 400,
               let failure = try? JSONDecoder().decode(OAuthFailure.self, from: tokenData) {
                if failure.error == "authorization_pending" { continue }
                if failure.error == "slow_down" { interval += .seconds(5); continue }
            }
            guard (200..<300).contains(tokenResponse.statusCode) else {
                throw deviceLoginError(tokenData, status: tokenResponse.statusCode)
            }
            let fresh = try decodeToken(tokenData, previousRefresh: nil)
            try TokenVault.save(fresh, account: settings.storageKey)
            tokens = fresh
            return
        }
    }

    private func checkLogin(_ epoch: UUID) throws {
        try Task.checkCancellation()
        guard generation == epoch else { throw CancellationError() }
    }

    private struct OAuthFailure: Decodable { let error: String }

    private func deviceLoginError(_ data: Data, status: Int) -> ZooError {
        let code = (try? JSONDecoder().decode(OAuthFailure.self, from: data))?.error
        let message = switch code {
        case "access_denied": "ログインが拒否されました。必要な場合はもう一度ログインしてください。"
        case "expired_token": "確認コードの期限が切れました。ログインし直してください。"
        case "unauthorized_client", "unsupported_grant_type": "AuthZ で iOS 用クライアントのデバイスコード認証を許可してください。"
        case "invalid_client": "iOS 用クライアント ID と、公開クライアントが有効であることを確認してください。"
        case "invalid_scope": "AuthZ で openid・profile・offline_access の利用を許可してください。"
        case "invalid_target": "AuthZ で iOS 用クライアントに Zoo API へのアクセスを許可してください。"
        default: "デバイス認証に失敗しました（HTTP \(status)）。ログインし直してください。"
        }
        return .auth(message)
    }

    private func postForm(endpoint: String, fields: [String: String], timeout: TimeInterval = 30) async throws -> (Data, HTTPURLResponse) {
        var request = URLRequest(url: try ConnectionSettings.secureURL(endpoint))
        request.httpMethod = "POST"
        request.timeoutInterval = timeout
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = OAuthForm.form(fields)
        return try await transport.send(request, timeout: .seconds(timeout))
    }

    private func exchangeToken(endpoint: String, fields: [String: String], previousRefresh: String?) async throws -> TokenSet {
        let (data, response) = try await postForm(endpoint: endpoint, fields: fields)
        try HTTPTransport.check(data, response)
        return try decodeToken(data, previousRefresh: previousRefresh)
    }

    private func decodeToken(_ data: Data, previousRefresh: String?) throws -> TokenSet {
        struct Response: Decodable {
            let access_token: String
            let refresh_token: String?
            let expires_in: Double
            let token_type: String
        }
        let decoded = try JSONDecoder().decode(Response.self, from: data)
        guard !decoded.access_token.isEmpty, decoded.token_type.lowercased() == "bearer",
              decoded.expires_in.isFinite, decoded.expires_in > 0 else {
            throw ZooError.invalidResponse
        }
        return TokenSet(access: decoded.access_token, refresh: decoded.refresh_token ?? previousRefresh,
                        expires: Date().addingTimeInterval(decoded.expires_in))
    }

    func accessToken() async throws -> String {
        try Task.checkCancellation()
        guard let tokens else { throw ZooError.expiredSession }
        if tokens.expires.timeIntervalSinceNow > 30 { return tokens.access }
        let epoch = generation
        let task: Task<TokenSet, Error>
        if let refreshTask {
            task = refreshTask
        } else {
            guard let refresh = tokens.refresh else { throw ZooError.expiredSession }
            task = Task {
                defer { if epoch == generation { refreshTask = nil } }
                let discovery = try await self.discovery()
                try Task.checkCancellation()
                var fields = ["grant_type": "refresh_token", "client_id": settings.clientID, "refresh_token": refresh]
                if !settings.resource.isEmpty { fields["resource"] = settings.resource }
                let fresh = try await self.exchangeToken(endpoint: discovery.token_endpoint, fields: fields, previousRefresh: refresh)
                try Task.checkCancellation()
                guard epoch == generation else { throw CancellationError() }
                // 呼び出し元がキャンセルされても、待機者へ返す前に更新後の認証情報を保存します。
                try TokenVault.save(fresh, account: settings.storageKey)
                self.tokens = fresh
                return fresh
            }
            refreshTask = task
        }
        do {
            let fresh = try await task.value
            try Task.checkCancellation()
            guard epoch == generation else { throw CancellationError() }
            return fresh.access
        } catch {
            try Task.checkCancellation()
            guard epoch == generation else { throw CancellationError() }
            if case ZooError.rejected(let status, _) = error, status == 400 || status == 401 {
                throw ZooError.expiredSession
            }
            throw error
        }
    }

    func logout() {
        generation = UUID()
        refreshTask?.cancel()
        refreshTask = nil
        tokens = nil
        TokenVault.delete(settings.storageKey)
    }
}
