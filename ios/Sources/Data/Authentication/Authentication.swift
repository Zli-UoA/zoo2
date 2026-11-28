import Foundation
import Security
import CryptoKit

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

    private static func logoutMarker(_ account: String) -> URL {
        let key = SHA256.hash(data: Data(account.utf8)).map { String(format: "%02x", $0) }.joined()
        return FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Zoo/LoggedOutSessions", isDirectory: true).appendingPathComponent(key)
    }

    private static func hasLogoutMarker(_ account: String) throws -> Bool {
        do {
            _ = try FileManager.default.attributesOfItem(atPath: logoutMarker(account).path)
            return true
        } catch let error as CocoaError where error.code == .fileNoSuchFile || error.code == .fileReadNoSuchFile {
            return false
        } catch {
            throw ZooError.auth("保存したログアウト状態を確認できませんでした。端末のロックを解除して再度お試しください。")
        }
    }

    static func load(_ account: String, deleteItem: (CFDictionary) -> OSStatus = SecItemDelete) throws -> TokenSet? {
        if try hasLogoutMarker(account) {
            try delete(account, deleteItem: deleteItem)
            return nil
        }
        var request = query(account)
        request[kSecReturnData as String] = true
        request[kSecReturnAttributes as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let item = result as? [String: Any], let data = item[kSecValueData as String] as? Data else {
            throw ZooError.auth("保存したログイン情報を読み取れませんでした。端末のロックを解除して再度お試しください。")
        }
        guard let token = try JSONDecoder().decode(TokenSet?.self, from: data) else {
            try delete(account, deleteItem: deleteItem)
            return nil
        }
        // 既存の保存項目も移行し、通話中に画面をロックしても更新トークンを保存できるようにする。
        if item[kSecAttrAccessible as String] as? String != kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String {
            try save(token, account: account)
        }
        return token
    }

    static func save(_ token: TokenSet, account: String) throws {
        if try hasLogoutMarker(account) { try delete(account) }
        let bytes = try JSONEncoder().encode(token)
        let attributes: [String: Any] = [kSecValueData as String: bytes,
                                        kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        var status = SecItemUpdate(query(account) as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var request = query(account)
            request.merge(attributes) { _, new in new }
            status = SecItemAdd(request as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw ZooError.auth("ログイン情報を安全に保存できませんでした。") }
    }

    static func delete(_ account: String, deleteItem: (CFDictionary) -> OSStatus = SecItemDelete,
                       updateItem: (CFDictionary, CFDictionary) -> OSStatus = SecItemUpdate,
                       writeMarker: (URL) throws -> Void = {
                           try FileManager.default.createDirectory(at: $0.deletingLastPathComponent(), withIntermediateDirectories: true)
                           try Data().write(to: $0, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
                       },
                       removeMarker: (URL) throws -> Void = { try FileManager.default.removeItem(at: $0) }) throws {
        let marker = logoutMarker(account)
        var markerError: Error?
        do { try writeMarker(marker) } catch { markerError = error }
        // ファイル保存と削除が同時に失敗しても、既存項目を無効化できれば旧セッションを復元しない。
        let invalidation = updateItem(query(account) as CFDictionary,
                                      [kSecValueData as String: Data("null".utf8)] as CFDictionary)
        // 終了直後に再起動しても旧トークンを復元せず、削除できるまで再試行する。
        let status = deleteItem(query(account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            if markerError != nil && invalidation != errSecSuccess && invalidation != errSecItemNotFound {
                throw ZooError.auth("ログイン情報の無効化・削除とログアウト状態の保存がすべて失敗しました。再起動後に以前のログイン状態が戻る可能性があります。端末を再起動せず、空き容量とロック状態を確認して再度お試しください。")
            }
            throw ZooError.auth("ログイン情報を削除できませんでした。端末のロックを解除し、再度ログアウトまたはログインしてください。")
        }
        do { try removeMarker(marker) }
        catch let error as CocoaError where error.code == .fileNoSuchFile || error.code == .fileReadNoSuchFile { }
        catch {
            throw ZooError.auth("認証情報は削除しましたが、ログアウト記録を削除できませんでした。端末のロック解除と空き容量の確認後に再試行してください。解消しない場合はZooを削除して再インストールしてください。端末内のキャッシュと下書きも削除されます。ログアウトで失われた下書きは、この画面では取り出せません。")
        }
    }
}

struct Discovery: Decodable, Sendable {
    let issuer: String
    let token_endpoint: String
    let device_authorization_endpoint: String?
    /// 再認証（E2EE の直前の再認証。認可コードフロー）に使う。
    let authorization_endpoint: String?
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
    private var loadedTokens = false
    private var refreshTask: Task<TokenSet, Error>?
    private(set) var refreshWaiterCount = 0
    private var pendingTokens: TokenSet?
    private let saveToken: (TokenSet, String) throws -> Void
    private var generation = UUID()
    var sessionID: UUID { generation }

    init(settings: ConnectionSettings, transport: HTTPTransport = HTTPTransport(),
         saveToken: @escaping (TokenSet, String) throws -> Void = { try TokenVault.save($0, account: $1) }) {
        self.settings = settings
        self.transport = transport
        self.saveToken = saveToken
    }

    private func sessionTokens() throws -> TokenSet? {
        if !loadedTokens {
            tokens = try TokenVault.load(settings.storageKey)
            loadedTokens = true
        }
        return tokens
    }

    var hasSession: Bool { get throws { try sessionTokens() != nil } }

    func discovery() async throws -> Discovery {
        let issuer = try ConnectionSettings.secureURL(settings.issuer)
        let (data, response) = try await transport.send(URLRequest(url: issuer.appendingPathComponent(".well-known/openid-configuration")))
        guard (200..<300).contains(response.statusCode) else {
            throw ZooError.auth("認証サーバーの設定を取得できませんでした（HTTP \(response.statusCode)）。再度お試しください。")
        }
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
        try logout()
        let epoch = generation
        let discovery = try await discovery()
        try checkLogin(epoch)
        guard let endpoint = discovery.device_authorization_endpoint else {
            throw ZooError.auth("認証サーバーがデバイスコード認証に対応していません。")
        }
        var fields = ["client_id": settings.clientID, "scope": settings.scope]
        if !settings.resource.isEmpty { fields["resource"] = settings.resource }
        let (data, response) = try await postForm(endpoint: endpoint, fields: fields)
        let received = clock.now
        try checkLogin(epoch)
        guard (200..<300).contains(response.statusCode) else {
            throw deviceLoginError(data, status: response.statusCode)
        }
        let grant = try JSONDecoder().decode(DeviceAuthorizationResponse.self, from: data)
        let prompt = try grant.prompt(issuer: settings.issuer)
        let deadline = received.advanced(by: .seconds(grant.expires_in))
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

    /// 再認証（認可コードフロー）の失敗。デバイスコード認証の文言は使わない。今のログインは、無効にならない。
    private func reauthError(_ data: Data, status: Int) -> ZooError {
        let code = (try? JSONDecoder().decode(OAuthFailure.self, from: data))?.error
        let message = switch code {
        case "unsupported_grant_type", "unauthorized_client": "AuthZ で iOS 用クライアントの認可コード（authorization_code）グラントを許可してください。"
        case "invalid_client": "iOS 用クライアント ID と、公開クライアントが有効であることを確認してください。"
        case "invalid_scope": "AuthZ で iOS 用クライアントの openid・profile の利用を許可してください。"
        case "invalid_target": "AuthZ で iOS 用クライアントに Zoo API へのアクセスを許可してください。"
        case "invalid_grant": "再認証のコードを受け付けられませんでした（使用済み、期限切れ、または確認値の不一致）。もう一度再認証してください。"
        default: "再認証に失敗しました（HTTP \(status)）。もう一度お試しください。ログイン中の状態は変わりません。"
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

    /// 再認証の認可コードを、トークンエンドポイントで ID トークンに換える（E2EE の直前の再認証。#685）。
    /// 今のセッションのトークンは変えない。
    func exchangeReauthCode(_ code: String, verifier: String, redirectURI: String) async throws -> String {
        struct Response: Decodable { let id_token: String? }
        let discovery = try await discovery()
        let (data, response) = try await postForm(endpoint: discovery.token_endpoint, fields: [
            "grant_type": "authorization_code", "code": code, "code_verifier": verifier,
            "redirect_uri": redirectURI, "client_id": settings.clientID
        ])
        guard (200..<300).contains(response.statusCode) else { throw reauthError(data, status: response.statusCode) }
        guard let token = try? JSONDecoder().decode(Response.self, from: data).id_token, !token.isEmpty else {
            throw ZooError.invalidResponse
        }
        return token
    }

    private func exchangeToken(endpoint: String, fields: [String: String], previousRefresh: String?) async throws -> TokenSet {
        let (data, response) = try await postForm(endpoint: endpoint, fields: fields)
        if response.statusCode == 400,
           (try? JSONDecoder().decode(OAuthFailure.self, from: data))?.error == "invalid_grant" {
            throw ZooError.expiredSession
        }
        guard (200..<300).contains(response.statusCode) else {
            throw deviceLoginError(data, status: response.statusCode)
        }
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
        if let pendingTokens {
            try saveToken(pendingTokens, settings.storageKey)
            tokens = pendingTokens
            self.pendingTokens = nil
        }
        guard let tokens = try sessionTokens() else { throw ZooError.expiredSession }
        if tokens.expires.timeIntervalSinceNow > 30 || (tokens.refresh == nil && tokens.expires > .now) { return tokens.access }
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
                pendingTokens = fresh
                try saveToken(fresh, settings.storageKey)
                self.tokens = fresh
                pendingTokens = nil
                return fresh
            }
            refreshTask = task
        }
        refreshWaiterCount += 1
        defer { refreshWaiterCount -= 1 }
        do {
            let fresh = try await task.value
            try Task.checkCancellation()
            guard epoch == generation else { throw CancellationError() }
            return fresh.access
        } catch {
            try Task.checkCancellation()
            guard epoch == generation else { throw CancellationError() }
            throw error
        }
    }

    func logout() throws {
        generation = UUID()
        refreshTask?.cancel()
        refreshTask = nil
        pendingTokens = nil
        tokens = nil
        loadedTokens = true
        try TokenVault.delete(settings.storageKey)
    }
}
