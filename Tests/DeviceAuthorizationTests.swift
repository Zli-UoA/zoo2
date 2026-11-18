import Foundation
import Synchronization
import Testing
@testable import Zoo

// URLProtocol の非同期コールバックを停止と競合させないため、タスク状態を Mutex で保護する。
private final class StreamingTokenProtocol: URLProtocol, @unchecked Sendable {
    private let loading = Mutex((task: Optional<Task<Void, Never>>.none, stopped: false))
    override class func canInit(with request: URLRequest) -> Bool { request.url?.path == "/token" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let task = Task {
            guard let url = request.url,
                  let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: nil) else { return }
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            do {
                while true {
                    try await Task.sleep(for: .milliseconds(20))
                    // 受信は続くが、トークン応答は完了しない状況を再現する。
                    client?.urlProtocol(self, didLoad: Data(" ".utf8))
                }
            } catch { }
        }
        let stopped = loading.withLock {
            if !$0.stopped { $0.task = task }
            return $0.stopped
        }
        if stopped { task.cancel() }
    }

    override func stopLoading() {
        let task = loading.withLock {
            $0.stopped = true
            defer { $0.task = nil }
            return $0.task
        }
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
        defer { auth.logout(); transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
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
        return Dictionary(uniqueKeysWithValues: (URLComponents(string: "https://test.invalid/?" + body)?.queryItems ?? [])
            .map { ($0.name, $0.value ?? "") })
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
            #expect(TokenVault.load(settings.storageKey)?.refresh == "approved-refresh")
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
        try await withAuthentication { auth in
            await #expect(throws: ZooError.self) { try await auth.login(clock: clock) { _ in } }
            #expect(clock.waits == [.seconds(5), .seconds(5), .seconds(2)])
            #expect(attempts.withLock { $0 } == 2)
            #expect(TokenVault.load(settings.storageKey) == nil)
        }
    }

    @Test("通信中に期限を過ぎた応答は保存しない")
    func expiresDuringResponse() async throws {
        let clock = AdvancingClock()
        stub { request in
            switch request.url?.path {
            case "/.well-known/openid-configuration": return Self.discovery
            case "/device/code": return try Self.grant(["expires_in": .number(10)])
            default:
                #expect(request.timeoutInterval == 5)
                clock.advance(by: .seconds(6))
                return Self.token
            }
        }
        try await withAuthentication { auth in
            await #expect(throws: ZooError.self) { try await auth.login(clock: clock) { _ in } }
            #expect(!auth.hasSession)
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
            #expect(auth.hasSession)
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
        try await withAuthentication { auth in
            await #expect(throws: ZooError.self) { try await auth.login(clock: clock) { _ in } }
            #expect(clock.waits == [.seconds(5), .seconds(25)])
            #expect(attempts.withLock { $0 } == 1)
            #expect(!auth.hasSession)
        }
    }

    @Test("データが届き続けても有効期限で通信と承認待ちを終了する")
    func streamingResponseCannotExtendDeadline() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StreamingTokenProtocol.self, StubProtocol.self]
        config.httpAdditionalHeaders = ["X-Zoo-Test-ID": stubID]
        let transport = HTTPTransport(session: URLSession(configuration: config))
        let auth = Authentication(settings: settings, transport: transport)
        defer { auth.logout(); transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        stub { request in
            if request.url?.path == "/.well-known/openid-configuration" { return Self.discovery }
            return try Self.grant(["expires_in": .number(2), "interval": .number(1)])
        }
        let started = ContinuousClock.now
        await #expect(throws: ZooError.self) { try await auth.login { _ in } }
        #expect(started.duration(to: .now) < .seconds(5))
        #expect(!auth.hasSession)
        #expect(TokenVault.load(settings.storageKey) == nil)
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
        try await withAuthentication { auth in
            await #expect(throws: ZooError.self) { try await auth.login(clock: AdvancingClock()) { _ in } }
            #expect(attempts.withLock { $0 } == 1)
            #expect(!auth.hasSession)
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
        try await withAuthentication { auth in
            let task = Task { try await auth.login { _ in shown.continuation.yield(()) } }
            var iterator = shown.stream.makeAsyncIterator()
            _ = await iterator.next()
            task.cancel()
            await #expect(throws: CancellationError.self) { try await task.value }
            #expect(TokenVault.load(settings.storageKey) == nil)
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
        try await withAuthentication { auth in
            let task = Task { try await auth.login(clock: AdvancingClock()) { _ in } }
            var iterator = started.stream.makeAsyncIterator()
            _ = await iterator.next()
            auth.logout()
            release.continuation.yield(())
            await #expect(throws: CancellationError.self) { try await task.value }
            #expect(!auth.hasSession)
            #expect(TokenVault.load(settings.storageKey) == nil)
        }
    }

    @Test("キャンセル直後の再試行を古い接続タスクが妨げない")
    func appSessionCanRetryImmediatelyAfterCancellation() async throws {
        let transport = transport()
        let session = AppSession(settings: settings, transport: transport, notificationStore: PreviewPushReceiptStore())
        defer { session.logout(); transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        let requested = AsyncStream<Void>.makeStream()
        defer { requested.continuation.finish() }
        stub { _ in
            requested.continuation.yield(())
            return (400, [:], Data())
        }
        session.startLogin()
        session.cancelLogin()
        session.startLogin()
        var iterator = requested.stream.makeAsyncIterator()
        _ = await iterator.next()
        session.cancelLogin()
        #expect(!session.connecting)
        #expect(session.deviceAuthorization == nil)
        #expect(session.error == nil)
    }
}
