import Foundation
import Synchronization
import Testing
@testable import Zoo

// URLProtocol の非同期コールバックを停止と競合させないため、タスク状態を Mutex で保護する。
private final class StreamingTokenProtocol: URLProtocol, @unchecked Sendable {
    static let chunks = Mutex(0)
    private let callbackLock = NSRecursiveLock()
    private let loading = Mutex((task: Optional<Task<Void, Never>>.none, stopped: false))
    override class func canInit(with request: URLRequest) -> Bool { request.url?.path == "/token" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let task = Task {
            guard let url = request.url,
                  let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: nil) else { return }
            deliver { client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed) }
            do {
                while true {
                    try await Task.sleep(for: .milliseconds(20))
                    // 受信は続くが、トークン応答は完了しない状況を再現する。
                    deliver {
                        Self.chunks.withLock { $0 += 1 }
                        client?.urlProtocol(self, didLoad: Data(" ".utf8))
                    }
                }
            } catch { }
        }
        let stopped = loading.withLock {
            if !$0.stopped { $0.task = task }
            return $0.stopped
        }
        if stopped { task.cancel() }
    }

    private func deliver(_ callback: () -> Void) {
        // URLProtocol の終了通知から stopLoading が再入しても、以降の通知を止める。
        callbackLock.withLock {
            guard !loading.withLock({ $0.stopped }) else { return }
            callback()
        }
    }

    override func stopLoading() {
        let task = callbackLock.withLock { loading.withLock {
            $0.stopped = true
            defer { $0.task = nil }
            return $0.task
        } }
        task?.cancel()
    }
}

private final class AdvancingClock: Clock, Sendable {
    typealias Instant = ContinuousClock.Instant
    typealias Duration = Swift.Duration
    private let state = Mutex((now: ContinuousClock.now, waits: [Duration]()))
    var now: Instant { state.withLock { $0.now } }
    var minimumResolution: Duration { .nanoseconds(1) }
    var waits: [Duration] { state.withLock { $0.waits } }

    func advance(by duration: Duration) { state.withLock { $0.now += duration } }

    func sleep(until deadline: Instant, tolerance: Duration?) async throws {
        try Task.checkCancellation()
        state.withLock {
            $0.waits.append($0.now.duration(to: deadline))
            $0.now = max($0.now, deadline)
        }
    }
}

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct DeviceAuthorizationTests {
    @Test("確認URLのホスト名は大文字と小文字を区別しない")
    func acceptsEquivalentHostCase() throws {
        let response = DeviceAuthorizationResponse(device_code: "test-device", user_code: "TEST-CODE",
            verification_uri: "https://ID.TEST.INVALID/device",
            verification_uri_complete: "https://Id.Test.Invalid/device?user_code=TEST-CODE", expires_in: 600, interval: 5)
        let prompt = try response.prompt(issuer: "https://id.test.invalid/auth")
        #expect(prompt.verificationURL.absoluteString == "https://Id.Test.Invalid/device?user_code=TEST-CODE")
    }

    private let stubID = UUID().uuidString
    private let settings = ConnectionSettings(server: "https://api.test.invalid", issuer: "https://id.test.invalid",
                                               clientID: UUID().uuidString, resource: "https://api.test.invalid",
                                               scope: "openid profile offline_access")

    private func transport() -> HTTPTransport {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StubProtocol.self]
        config.httpAdditionalHeaders = ["X-Zoo-Test-ID": stubID]
        return HTTPTransport(session: URLSession(configuration: config))
    }

    private func withAuthentication(_ action: (Authentication) async throws -> Void) async throws {
        let transport = transport()
        let auth = Authentication(settings: settings, transport: transport)
        defer { try? auth.logout(); transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        try await action(auth)
    }

    private func stub(_ handler: @escaping StubProtocol.Handler) { StubProtocol.install(handler, id: stubID) }

    nonisolated private static var discovery: (Int, [String: String], Data) {
        (200, [:], Data(#"{"issuer":"https://id.test.invalid","device_authorization_endpoint":"https://id.test.invalid/device/code","token_endpoint":"https://id.test.invalid/token"}"#.utf8))
    }

    nonisolated private static var token: (Int, [String: String], Data) {
        (200, [:], Data(#"{"access_token":"approved-access","refresh_token":"approved-refresh","expires_in":3600,"token_type":"Bearer"}"#.utf8))
    }

    nonisolated private static func grant(_ overrides: [String: JSONValue] = [:], complete: Bool = true) throws -> (Int, [String: String], Data) {
        var fields: [String: JSONValue] = ["device_code": .string("private+code&value"), "user_code": .string("ABCD-EFGH"),
            "verification_uri": .string("https://id.test.invalid/device"), "expires_in": .number(600)]
        if complete { fields["verification_uri_complete"] = .string("https://id.test.invalid/device?user_code=ABCD-EFGH") }
        fields.merge(overrides) { _, new in new }
        return (200, [:], try JSONEncoder().encode(fields))
    }

    nonisolated private static func form(_ request: URLRequest) throws -> [String: String] {
        let body = String(decoding: try StubProtocol.body(request), as: UTF8.self)
        var fields: [String: String] = [:]
        for pair in body.split(separator: "&") {
            let parts = pair.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            let name = try #require(String(parts[0]).replacingOccurrences(of: "+", with: " ").removingPercentEncoding)
            let value = try #require((parts.count == 2 ? String(parts[1]) : "").replacingOccurrences(of: "+", with: " ").removingPercentEncoding)
            fields[name] = value
        }
        return fields
    }

    @Test("承認待ちと低速化に従ってトークンを取得し、再起動後も復元できる")
    func pendingSlowDownAndSuccess() async throws {
        let clock = AdvancingClock()
        let attempts = Mutex(0)
        let clientID = settings.clientID
        stub { request in
            #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code":
                #expect(request.httpMethod == "POST")
                #expect(request.value(forHTTPHeaderField: "Content-Type") == "application/x-www-form-urlencoded")
                #expect(try Self.form(request) == ["client_id": clientID, "scope": "openid profile offline_access", "resource": "https://api.test.invalid"])
                return try Self.grant()
            default:
                #expect(request.url?.path == "/token")
                #expect(try Self.form(request) == ["client_id": clientID, "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                                                  "device_code": "private+code&value", "resource": "https://api.test.invalid"])
                let attempt = attempts.withLock { $0 += 1; return $0 }
                switch attempt {
                case 1, 3: return (400, [:], Data(#"{"error":"authorization_pending"}"#.utf8))
                case 2: return (400, [:], Data(#"{"error":"slow_down"}"#.utf8))
                default: return Self.token
                }
            }
        }
        try await withAuthentication { auth in
            var shown: [DeviceAuthorizationPrompt] = []
            try await auth.login(clock: clock) { shown.append($0) }
            #expect(shown.count == 1)
            #expect(shown.first?.userCode == "ABCD-EFGH")
            #expect(shown.first?.verificationURL.absoluteString == "https://id.test.invalid/device?user_code=ABCD-EFGH")
            #expect(clock.waits == [.seconds(5), .seconds(5), .seconds(10), .seconds(10)])
            #expect(try TokenVault.load(settings.storageKey)?.refresh == "approved-refresh")
            let restored = Authentication(settings: settings, transport: auth.transport)
            #expect(try await restored.accessToken() == "approved-access")
        }
    }

    @Test("完全な確認URLがない場合も、通常の確認URLと指定間隔を使う")
    func verificationFallbackAndExplicitInterval() async throws {
        let clock = AdvancingClock()
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code": return try Self.grant(["interval": .number(2)], complete: false)
            default: return Self.token
            }
        }
        try await withAuthentication { auth in
            try await auth.login(clock: clock) {
                #expect($0.verificationURL.absoluteString == "https://id.test.invalid/device")
            }
            #expect(clock.waits == [.seconds(2)])
        }
    }

    @Test("期限に達した後はトークン要求を送らない")
    func expiresWhilePending() async throws {
        let clock = AdvancingClock()
        let attempts = Mutex(0)
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code": return try Self.grant(["expires_in": .number(12)])
            default:
                attempts.withLock { $0 += 1 }
                return (400, [:], Data(#"{"error":"authorization_pending"}"#.utf8))
            }
        }
        try await withAuthentication { (auth: Authentication) async throws in
            await #expect(throws: ZooError.self) { try await auth.login(clock: clock) { _ in } }
            #expect(clock.waits == [.seconds(5), .seconds(5), .seconds(2)])
            #expect(attempts.withLock { $0 } == 2)
            #expect(try TokenVault.load(settings.storageKey) == nil)
        }
    }

    @Test("コード発行の待機時間を受信後の有効期間から引かない")
    func delayedGrantKeepsLifetime() async throws {
        let clock = AdvancingClock()
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code":
                clock.advance(by: .seconds(20))
                return try Self.grant(["expires_in": .number(10)])
            default: return Self.token
            }
        }
        try await withAuthentication { auth in
            try await auth.login(clock: clock) { _ in }
            #expect(try auth.hasSession)
            #expect(clock.waits == [.seconds(5)])
        }
    }

    @Test("通信中に期限を過ぎた応答は保存しない")
    func expiresDuringResponse() async throws {
        let clock = AdvancingClock()
        let receivedToken = Mutex(false)
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code": return try Self.grant(["expires_in": .number(10)])
            default:
                #expect(request.url?.path == "/token")
                #expect(request.timeoutInterval == 5)
                clock.advance(by: .seconds(6))
                receivedToken.withLock { $0 = true }
                return Self.token
            }
        }
        try await withAuthentication { (auth: Authentication) async throws in
            await #expect(throws: ZooError.self) { try await auth.login(clock: clock) { _ in } }
            #expect(try !auth.hasSession)
            #expect(receivedToken.withLock { $0 })
        }
    }

    @Test("タイムアウト時は問い合わせ間隔を倍にする")
    func timeoutBacksOff() async throws {
        let clock = AdvancingClock()
        let attempts = Mutex(0)
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code": return try Self.grant()
            default:
                if attempts.withLock({ $0 += 1; return $0 }) == 1 { throw URLError(.timedOut) }
                return Self.token
            }
        }
        try await withAuthentication { auth in
            try await auth.login(clock: clock) { _ in }
            #expect(clock.waits == [.seconds(5), .seconds(10)])
        }
    }

    @Test("HTTP 429では指定待機時間と段階的な間隔拡大を守る", arguments: ["20", "不正な値", "-1"])
    func rateLimitBacksOff(retryAfter: String) async throws {
        let clock = AdvancingClock()
        let attempts = Mutex(0)
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code": return try Self.grant()
            default:
                switch attempts.withLock({ $0 += 1; return $0 }) {
                case 1: return (429, ["Retry-After": retryAfter], Data())
                case 2: return (429, [:], Data(#"{"error":"slow_down"}"#.utf8))
                default: return Self.token
                }
            }
        }
        try await withAuthentication { auth in
            try await auth.login(clock: clock) { _ in }
            #expect(clock.waits == (retryAfter == "20"
                ? [.seconds(5), .seconds(20), .seconds(40)]
                : [.seconds(5), .seconds(10), .seconds(20)]))
            #expect(try auth.hasSession)
        }
    }

    @Test("Retry-Afterが期限より先でも、期限後は問い合わせない", arguments: ["60", "Sat, 01 Jan 2050 00:00:00 GMT", "1e300"])
    func rateLimitCannotExtendDeadline(retryAfter: String) async throws {
        let clock = AdvancingClock()
        let attempts = Mutex(0)
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code": return try Self.grant(["expires_in": .number(30)])
            default:
                if attempts.withLock({ $0 += 1; return $0 }) == 1 {
                    return (429, ["Retry-After": retryAfter], Data())
                }
                return Self.token
            }
        }
        try await withAuthentication { (auth: Authentication) async throws in
            await #expect(throws: ZooError.self) { try await auth.login(clock: clock) { _ in } }
            #expect(clock.waits == [.seconds(5), .seconds(25)])
            #expect(attempts.withLock { $0 } == 1)
            #expect(try !auth.hasSession)
        }
    }

    @Test("データが届き続けても有効期限で通信と承認待ちを終了する")
    func streamingResponseCannotExtendDeadline() async throws {
        StreamingTokenProtocol.chunks.withLock { $0 = 0 }
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StreamingTokenProtocol.self, StubProtocol.self]
        config.httpAdditionalHeaders = ["X-Zoo-Test-ID": stubID]
        let transport = HTTPTransport(session: URLSession(configuration: config))
        let auth = Authentication(settings: settings, transport: transport)
        defer { try? auth.logout(); transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        stub { request in
            if request.url?.path == "/.well-known/openid-configuration" { return Self.discovery }
            return try Self.grant(["expires_in": .number(2), "interval": .number(1)])
        }
        let started = ContinuousClock.now
        await #expect(throws: ZooError.self) { try await auth.login { _ in } }
        #expect(started.duration(to: .now) < .seconds(5))
        #expect(StreamingTokenProtocol.chunks.withLock { $0 } > 0)
        #expect(try !auth.hasSession)
        #expect(try TokenVault.load(settings.storageKey) == nil)
    }

    @Test("拒否・失効・設定エラーで待機を終了する", arguments: ["access_denied", "expired_token", "invalid_client", "unauthorized_client", "invalid_scope", "invalid_target"])
    func oauthFailureStopsPolling(code: String) async throws {
        let attempts = Mutex(0)
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code": return try Self.grant()
            default:
                attempts.withLock { $0 += 1 }
                return (400, [:], try JSONEncoder().encode(["error": code]))
            }
        }
        try await withAuthentication { (auth: Authentication) async throws in
            await #expect(throws: ZooError.self) { try await auth.login(clock: AdvancingClock()) { _ in } }
            #expect(attempts.withLock { $0 } == 1)
            #expect(try !auth.hasSession)
        }
    }

    @Test("別オリジンや危険な確認URLを表示しない", arguments: [
        "http://id.test.invalid/device", "https://evil.test.invalid/device", "https://id.test.invalid:444/device",
        "https://user@id.test.invalid/device", "https://id.test.invalid/device#fragment", "relative/path"
    ])
    func rejectsUntrustedVerificationURL(url: String) async throws {
        stub { request in
            if request.url?.path == "/.well-known/openid-configuration" { return Self.discovery }
            #expect(request.url?.path == "/device/code")
            return try Self.grant(["verification_uri_complete": .string(url)])
        }
        try await withAuthentication { auth in
            await #expect(throws: ZooError.self) {
                try await auth.login(clock: AdvancingClock()) { _ in Issue.record("不正な確認URLが表示されました。") }
            }
        }
    }

    @Test("空のコードと不正な有効期限・間隔を拒否する", arguments: [
        ["device_code": JSONValue.string("")], ["user_code": .string(" ")],
        ["expires_in": .number(0)], ["expires_in": .number(-1)], ["interval": .number(0)], ["interval": .number(-1)]
    ])
    func rejectsInvalidGrant(overrides: [String: JSONValue]) async throws {
        stub { request in
            if request.url?.path == "/.well-known/openid-configuration" { return Self.discovery }
            #expect(request.url?.path == "/device/code")
            return try Self.grant(overrides)
        }
        try await withAuthentication { auth in
            await #expect(throws: ZooError.self) { try await auth.login(clock: AdvancingClock()) { _ in Issue.record("不正なコードが表示されました。") } }
        }
    }

    @Test("未対応のDiscoveryでは設定エラーを表示する")
    func requiresDeviceEndpoint() async throws {
        stub { request in
            #expect(request.url?.path == "/.well-known/openid-configuration")
            return (200, [:], Data(#"{"issuer":"https://id.test.invalid","token_endpoint":"https://id.test.invalid/token"}"#.utf8))
        }
        try await withAuthentication { auth in
            await #expect(throws: ZooError.self) { try await auth.login { _ in Issue.record("未対応のコードが表示されました。") } }
        }
    }

    @Test("承認待ちのキャンセルで問い合わせと保存を止める")
    func cancellationStopsPolling() async throws {
        let shown = AsyncStream<Void>.makeStream()
        defer { shown.continuation.finish() }
        stub { request in
            if request.url?.path == "/.well-known/openid-configuration" { return Self.discovery }
            #expect(request.url?.path == "/device/code")
            return try Self.grant()
        }
        try await withAuthentication { (auth: Authentication) async throws in
            let task = Task { try await auth.login { _ in shown.continuation.yield(()) } }
            var iterator = shown.stream.makeAsyncIterator()
            _ = await iterator.next()
            task.cancel()
            await #expect(throws: CancellationError.self) { try await task.value }
            #expect(try TokenVault.load(settings.storageKey) == nil)
        }
    }

    @Test("ログアウト後に到着する承認応答を保存しない")
    func logoutDiscardsInFlightApproval() async throws {
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code": return try Self.grant()
            default:
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                return Self.token
            }
        }
        try await withAuthentication { (auth: Authentication) async throws in
            let task = Task { try await auth.login(clock: AdvancingClock()) { _ in } }
            var iterator = started.stream.makeAsyncIterator()
            _ = await iterator.next()
            try? auth.logout()
            release.continuation.yield(())
            await #expect(throws: CancellationError.self) { try await task.value }
            #expect(try !auth.hasSession)
            #expect(try TokenVault.load(settings.storageKey) == nil)
        }
    }

    @Test("キャンセル直後の再試行を古い接続タスクが妨げない")
    func appSessionCanRetryImmediatelyAfterCancellation() async throws {
        let transport = transport()
        let session = AppSession(settings: settings, transport: transport, notificationStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys())
        defer { session.logout(); transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        let destination = try #require(PushDestination(payload: ["user_id": UUID().uuidString, "channel_id": UUID().uuidString]))
        session.notifications.opened(.message(destination))
        let requested = AsyncStream<Void>.makeStream()
        defer { requested.continuation.finish() }
        stub { _ in
            requested.continuation.yield(())
            return (400, [:], Data())
        }
        session.startLogin()
        #expect(session.notifications.destination == destination)
        session.cancelLogin()
        session.startLogin()
        var iterator = requested.stream.makeAsyncIterator()
        _ = await iterator.next()
        session.cancelLogin()
        #expect(!session.connecting)
        #expect(session.deviceAuthorization == nil)
        #expect(session.error == nil)
    }

    @Test("アカウント切替で通知センターの本文を消去し、既にタップされた遷移先は本人確認まで保持する", arguments: [false, true])
    func discardingAccountNotificationsRemovesDeliveredPreviews(hasReceipt: Bool) async throws {
        let transport = transport()
        defer { transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        let receipts = PreviewPushReceiptStore(), keys = InMemoryNotificationPreviewKeys()
        let user = UUID().uuidString
        if hasReceipt {
            let receipt = PushReceipt(server: settings.server, account: PushReceipt.digest(settings.storageKey + "|" + user),
                                      token: "aabb", environment: "sandbox", secret: "解除用テスト値")
            try receipts.save(receipt)
            keys.save(NotificationPreviewKey(userID: user, secret: receipt.secret))
        }
        stub { request in
            #expect(request.url?.path == "/api/push/apns/revoke")
            throw URLError(.notConnectedToInternet)
        }
        let controller = NotificationController(transport: transport, receiptStore: receipts, previewKeys: keys)
        let target = try #require(PushDestination(payload: ["user_id": user, "channel_id": UUID().uuidString]))
        controller.opened(.message(target))
        var delivered = ["以前の通知本文"]
        let revocation = controller.discardAccountNotifications { delivered.removeAll() }
        #expect(keys.load() == nil)
        #expect(delivered.isEmpty)
        #expect(controller.destination == target)
        await revocation.value
    }

    @Test("復元の一時エラーでは通知を維持し、手動ログインの開始時に旧利用者の鍵を即時削除する")
    func failedRestoreKeepsPreviewUntilInteractiveLogin() async throws {
        let transport = transport()
        let receipts = PreviewPushReceiptStore()
        let keys = InMemoryNotificationPreviewKeys()
        let user = UUID().uuidString
        let receipt = PushReceipt(server: settings.server, account: PushReceipt.digest(settings.storageKey + "|" + user), token: "aabb",
                                  environment: "sandbox", secret: "解除用テスト値")
        let original = NotificationPreviewKey(userID: user, secret: receipt.secret)
        keys.save(original)
        try receipts.save(receipt)
        try TokenVault.save(TokenSet(access: "test-access", refresh: nil, expires: .now.addingTimeInterval(3600)), account: settings.storageKey)
        defer { try? TokenVault.delete(settings.storageKey); transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        let paths = Mutex<[String]>([])
        stub { request in
            paths.withLock { $0.append(request.url?.path ?? "") }
            return (503, [:], Data())
        }
        let session = AppSession(settings: settings, transport: transport, notificationStore: receipts, previewKeys: keys)
        session.notifications.received(token: Data([0xaa, 0xbb]))
        await session.resume()
        await session.resume()
        #expect(session.error != nil)
        #expect(paths.withLock { $0 } == ["/api/users/me", "/api/users/me"])
        #expect(try receipts.load() == receipt)
        #expect(keys.load() == original)

        let requests = AsyncStream<String>.makeStream()
        defer { requests.continuation.finish() }
        stub { request in
            requests.continuation.yield(request.url?.path ?? "")
            throw URLError(.notConnectedToInternet)
        }
        session.startLogin()
        #expect(keys.load() == nil)
        var iterator = requests.stream.makeAsyncIterator()
        let first = await iterator.next(), second = await iterator.next()
        #expect(Set([first, second].compactMap { $0 }) == ["/api/push/apns/revoke", "/.well-known/openid-configuration"])
        #expect(try TokenVault.load(settings.storageKey) == nil)
        #expect(try receipts.load()?.revocationPending == true)
        #expect(keys.load() == nil)
    }

    @Test("ログアウト済みの起動で失敗した端末登録の解除を次の前面復帰時に再試行する")
    func signedOutResumeRetriesRevocation() async throws {
        let transport = transport()
        let receipts = PreviewPushReceiptStore()
        let receipt = PushReceipt(server: settings.server, account: "利用者", token: "aabb",
                                  environment: "sandbox", secret: "解除用テスト値")
        try receipts.save(receipt)
        defer { transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        let attempts = Mutex(0)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        stub { request in
            #expect(request.url?.path == "/api/push/apns/revoke")
            #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
            let attempt = attempts.withLock { $0 += 1; return $0 }
            if attempt == 1 {
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                throw URLError(.notConnectedToInternet)
            }
            return (200, [:], Data(#"{"ok":true}"#.utf8))
        }
        let session = AppSession(settings: settings, transport: transport, notificationStore: receipts, previewKeys: InMemoryNotificationPreviewKeys())
        #expect(session.isStarting)
        let resuming = Task { await session.resume() }
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(!session.isStarting)
        #expect(!session.restoring)
        release.continuation.yield(())
        await resuming.value
        #expect(try receipts.load()?.secret == receipt.secret)
        #expect(try receipts.load()?.revocationPending == true)
        await session.resume()
        #expect(try receipts.load() == nil)
        #expect(attempts.withLock { $0 } == 2)
    }

    @Test("未ログイン時の通知解除が終わるまで新しいログインを開始しない")
    func loginWaitsForSignedOutCleanup() async throws {
        let transport = transport(), receipts = PreviewPushReceiptStore()
        try receipts.save(PushReceipt(server: settings.server, account: "利用者", token: "aabb",
                                      environment: "sandbox", secret: "解除用テスト値"))
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        let discoveryCount = Mutex(0)
        let session = AppSession(settings: settings, transport: transport, notificationStore: receipts,
                                 previewKeys: InMemoryNotificationPreviewKeys())
        defer {
            release.continuation.finish(); started.continuation.finish(); session.logout()
            transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID)
        }
        stub { request in
            if request.url?.path == "/api/push/apns/revoke" {
                started.continuation.yield(())
                for await _ in release.stream { break }
                return (200, [:], Data(#"{"ok":true}"#.utf8))
            }
            #expect(request.url?.path == "/.well-known/openid-configuration")
            discoveryCount.withLock { $0 += 1 }
            return (400, [:], Data())
        }
        let resuming = Task {
            defer { started.continuation.finish() }
            await session.resume()
        }
        defer { resuming.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        session.startLogin()
        // ログインの別タスクにも実行機会を与え、解除待ちを飛ばす回帰を検出する。
        try await Task.sleep(for: .milliseconds(50))
        #expect(session.connecting)
        #expect(discoveryCount.withLock { $0 } == 0)
        release.continuation.finish()
        await resuming.value
        let deadline = ContinuousClock.now + .seconds(2)
        while session.connecting && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        #expect(!session.connecting)
        #expect(discoveryCount.withLock { $0 } == 1)
    }

    @Test("古い起動のキャッシュ読出し失敗は新しいログインのエラーを上書きしない")
    func staleResumeDoesNotOverwriteLoginError() async throws {
        let transport = transport(), cache = try await SyncCache.open(inMemory: true)
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        let opens = Mutex(0), requests = Mutex(0)
        let session = AppSession(settings: settings, transport: transport, notificationStore: PreviewPushReceiptStore(),
                                 previewKeys: InMemoryNotificationPreviewKeys(), openSyncCache: {
            let attempt = opens.withLock { $0 += 1; return $0 }
            if attempt == 1 {
                started.continuation.yield(())
                for await _ in release.stream { break }
                throw CocoaError(.fileReadCorruptFile)
            }
            return cache
        })
        defer {
            release.continuation.finish(); started.continuation.finish(); session.logout()
            transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID)
        }
        stub { _ in requests.withLock { $0 += 1 }; return (400, [:], Data()) }
        let resuming = Task {
            defer { started.continuation.finish() }
            await session.resume()
        }
        defer { resuming.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        session.startLogin()
        let deadline = ContinuousClock.now + .seconds(2)
        while session.connecting && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        try #require(!session.connecting && requests.withLock { $0 } == 1)
        let loginError = try #require(session.error)
        #expect(loginError != CocoaError(.fileReadCorruptFile).localizedDescription)
        release.continuation.finish()
        await resuming.value
        #expect(session.error == loginError)
    }

    @Test("起動の後処理失敗は次のログインに一度だけ伝わり、その次の再試行は進む")
    func cleanupFailureIsConsumedOnce() async throws {
        let transport = transport(), saves = Mutex(0), requests = Mutex(0)
        let cache = try await SyncCache.open(inMemory: true, beforeSave: {
            let attempt = saves.withLock { $0 += 1; return $0 }
            if attempt == 1 { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let session = AppSession(settings: settings, transport: transport, notificationStore: PreviewPushReceiptStore(),
                                 previewKeys: InMemoryNotificationPreviewKeys(), syncCache: cache)
        defer { session.logout(); transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        stub { _ in requests.withLock { $0 += 1 }; return (400, [:], Data()) }
        await session.resume()
        #expect(session.error == CocoaError(.fileWriteOutOfSpace).localizedDescription)
        session.startLogin()
        var deadline = ContinuousClock.now + .seconds(2)
        while session.connecting && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        try #require(!session.connecting)
        #expect(session.error == CocoaError(.fileWriteOutOfSpace).localizedDescription)
        #expect(saves.withLock { $0 } == 1)
        #expect(requests.withLock { $0 } == 0)
        session.startLogin()
        deadline = ContinuousClock.now + .seconds(2)
        while session.connecting && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        #expect(!session.connecting)
        #expect(saves.withLock { $0 } == 2)
        #expect(requests.withLock { $0 } == 1)
    }

    @Test("復元した認証が失効していたら端末登録を解除する")
    func expiredRestoreRevokesPushReceipt() async throws {
        let transport = transport()
        let receipts = PreviewPushReceiptStore()
        try receipts.save(PushReceipt(server: settings.server, account: "利用者", token: "aabb",
                                     environment: "sandbox", secret: "解除用テスト値"))
        try TokenVault.save(TokenSet(access: "expired-access", refresh: nil, expires: .now.addingTimeInterval(3600)), account: settings.storageKey)
        let revoked = AsyncStream<Void>.makeStream()
        defer {
            revoked.continuation.finish()
            try? TokenVault.delete(settings.storageKey)
            transport.session.invalidateAndCancel()
            StubProtocol.install(nil, id: stubID)
        }
        stub { request in
            if request.url?.path == "/api/users/me" { return (401, [:], Data()) }
            #expect(request.url?.path == "/api/push/apns/revoke")
            revoked.continuation.yield(())
            return (200, [:], Data(#"{"ok":true}"#.utf8))
        }
        let session = AppSession(settings: settings, transport: transport, notificationStore: receipts, previewKeys: InMemoryNotificationPreviewKeys())
        await session.resume()
        var iterator = revoked.stream.makeAsyncIterator()
        _ = await iterator.next()
        await session.resume()
        #expect(try TokenVault.load(settings.storageKey) == nil)
        #expect(try receipts.load() == nil)
        #expect(session.userID == nil)
    }

    @Test("遅れている自動復元をキャンセルしても認証情報と端末登録を維持する")
    func cancellingRestorationKeepsPushReceipt() async throws {
        let transport = transport()
        let receipts = PreviewPushReceiptStore()
        let receipt = PushReceipt(server: settings.server, account: "利用者", token: "aabb",
                                  environment: "sandbox", secret: "解除用テスト値")
        try receipts.save(receipt)
        try TokenVault.save(TokenSet(access: "test-access", refresh: nil, expires: .now.addingTimeInterval(3600)), account: settings.storageKey)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer {
            started.continuation.finish()
            release.continuation.finish()
            try? TokenVault.delete(settings.storageKey)
            transport.session.invalidateAndCancel()
            StubProtocol.install(nil, id: stubID)
        }
        stub { request in
            #expect(request.url?.path == "/api/users/me")
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (401, [:], Data())
        }
        let session = AppSession(settings: settings, transport: transport, notificationStore: receipts, previewKeys: InMemoryNotificationPreviewKeys())
        let restoring = Task { await session.resume() }
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(session.restoring)
        session.cancelLogin()
        release.continuation.yield(())
        await restoring.value
        #expect(!session.isStarting)
        #expect(!session.restoring)
        #expect(!session.connecting)
        #expect(session.error == nil)
        #expect(try TokenVault.load(settings.storageKey)?.access == "test-access")
        #expect(try receipts.load() == receipt)
    }
}
