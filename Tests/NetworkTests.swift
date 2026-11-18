import Foundation
import Synchronization
import Testing
@testable import Zoo

// URLProtocolの継承にはunchecked適合が必要なため、追加する可変状態はすべてMutexで保護します。
final class StubProtocol: URLProtocol, @unchecked Sendable {
    typealias Handler = @Sendable (URLRequest) async throws -> (Int, [String: String], Data)
    private static let handlers = Mutex<[String: Handler]>([:])
    private let loading = Mutex((task: Optional<Task<Void, Never>>.none, stopped: false))

    static func install(_ handler: Handler?, id: String) {
        handlers.withLock { $0[id] = handler }
    }

    nonisolated static func body(_ request: URLRequest) throws -> Data {
        if let data = request.httpBody { return data }
        guard let stream = request.httpBodyStream else { throw URLError(.badServerResponse) }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var bytes = [UInt8](repeating: 0, count: 1024)
        while stream.hasBytesAvailable {
            let count = stream.read(&bytes, maxLength: bytes.count)
            if count < 0 { throw stream.streamError ?? URLError(.cannotDecodeRawData) }
            if count == 0 { break }
            data.append(contentsOf: bytes.prefix(count))
        }
        return data
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let task = Task {
            do {
                let id = request.value(forHTTPHeaderField: "X-Zoo-Test-ID") ?? ""
                let handler = Self.handlers.withLock { $0[id] }
                guard let handler else { throw URLError(.badServerResponse) }
                let (status, headers, data) = try await handler(request)
                try Task.checkCancellation()
                let url = try #require(request.url)
                let response = try #require(HTTPURLResponse(url: url, statusCode: status,
                                                           httpVersion: "HTTP/1.1", headerFields: headers))
                client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
                client?.urlProtocol(self, didLoad: data)
                client?.urlProtocolDidFinishLoading(self)
            } catch {
                guard !Task.isCancelled else { return }
                client?.urlProtocol(self, didFailWithError: error)
            }
        }
        let stopped = loading.withLock { state in
            if !state.stopped { state.task = task }
            return state.stopped
        }
        if stopped { task.cancel() }
    }
    override func stopLoading() {
        let task = loading.withLock { state in
            state.stopped = true
            defer { state.task = nil }
            return state.task
        }
        task?.cancel()
    }
}

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct NetworkTests {
    private let stubID = UUID().uuidString

    private func stub(_ handler: StubProtocol.Handler?) {
        StubProtocol.install(handler, id: stubID)
    }

    private func client(expired: Bool = false) throws -> APIClient {
        let settings = ConnectionSettings(server: "https://api.test.invalid", issuer: "https://id.test.invalid",
                                          clientID: UUID().uuidString, resource: "https://api.test.invalid", scope: "openid")
        try TokenVault.save(TokenSet(access: "test-access", refresh: "test-refresh", expires: Date().addingTimeInterval(expired ? -1 : 3600)), account: settings.storageKey)
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StubProtocol.self]
        config.httpAdditionalHeaders = ["X-Zoo-Test-ID": stubID]
        let transport = HTTPTransport(session: URLSession(configuration: config))
        let auth = Authentication(settings: settings, transport: transport)
        return APIClient(settings: settings, auth: auth, transport: transport)
    }

    @Test func testPushCleanupUsesOriginalServerWithoutBearerAndRejectsSPAResponse() async throws {
        let api = try client()
        defer { api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        let receipt = PushReceipt(server: "https://previous.test.invalid", account: "old", token: "aabb", environment: "sandbox", secret: String(repeating: "a", count: 64))
        let backend = LivePushBackend(api: api, transport: api.transport)
        stub { request in
            #expect(request.url?.host == "previous.test.invalid")
            #expect(request.url?.path == "/api/push/apns/revoke")
            #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
            let body = try #require(JSONSerialization.jsonObject(with: StubProtocol.body(request)) as? [String: String])
            #expect(body["id"] == receipt.id)
            #expect(body["revocation_secret"] == receipt.secret)
            return (200, [:], Data("<html>SPA のフォールバックページ</html>".utf8))
        }
        await #expect(throws: DecodingError.self) { try await backend.revoke(receipt) }
        stub { _ in (200, [:], Data(#"{"ok":true}"#.utf8)) }
        try await backend.revoke(receipt)
    }

    @Test func testPushRegistrationUsesEnvironmentConfigurationAndStableReceipt() async throws {
        let api = try client()
        defer { api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        let receipt = PushReceipt(server: api.settings.server, account: "user", token: "aabb", environment: "sandbox", secret: String(repeating: "a", count: 64))
        stub { request in
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            if request.url?.path == "/api/push/apns/config" {
                return (200, [:], Data(#"{"sandbox":true,"production":false}"#.utf8))
            }
            #expect(request.url?.path == "/api/push/apns/subscriptions")
            let body = try #require(JSONSerialization.jsonObject(with: StubProtocol.body(request)) as? [String: Any])
            #expect(body["created_at"] as? Int64 == receipt.createdAt)
            #expect(body["environment"] as? String == "sandbox")
            return (200, [:], try JSONSerialization.data(withJSONObject: ["id": receipt.id]))
        }
        try await LivePushBackend(api: api, transport: api.transport).subscribe(receipt)
    }

    @Test func testWritesUseBearerAndDecodeExistingBackendEnvelope() async throws {
        let api = try client()
        defer { api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        stub { request in
            #expect(request.url?.path == "/api/messages")
            #expect(request.httpMethod == "POST")
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            #expect(request.value(forHTTPHeaderField: "X-Dev-User") == nil)
            return (200, [:], Data(#"{"data":{"id":"m","user_id":"u","channel_id":"c","content":"こんにちは","created_at":"2026-10-26 10:00:00","parent_id":null,"deleted_at":null,"updated_at":null},"txid":"123"}"#.utf8))
        }
        let message: Message = try await api.write("api/messages", body: ["content": .string("こんにちは"), "channel_id": .string("c")])
        #expect(message.content == "こんにちは")
    }

    @Test func testUnauthorizedIsAnExpiredSession() async throws {
        let api = try client()
        defer { api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        stub { _ in (401, [:], Data()) }
        do { let _: Person = try await api.get("api/users/me"); Issue.record("認証エラーの応答が受け入れられました。") }
        catch ZooError.expiredSession { }
    }

    @Test func testRefreshIsSharedAcrossConcurrentRequests() async throws {
        let api = try client(expired: true)
        defer { api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        await confirmation("トークン交換が1回だけ行われる") { refreshed in
            stub { request in
                if request.url?.path == "/.well-known/openid-configuration" {
                    return Self.discoveryResponse
                }
                #expect(request.url?.path == "/token")
                refreshed()
                return Self.tokenResponse
            }
            async let first = api.auth.accessToken()
            async let second = api.auth.accessToken()
            do {
                let values = try await [first, second]
                #expect(values == ["renewed", "renewed"])
                #expect(TokenVault.load(api.settings.storageKey)?.refresh == "rotated")
            } catch { Issue.record(error) }
        }
    }

    nonisolated private static var discoveryResponse: (Int, [String: String], Data) {
        (200, [:], Data(#"{"issuer":"https://id.test.invalid","authorization_endpoint":"https://id.test.invalid/authorize","token_endpoint":"https://id.test.invalid/token"}"#.utf8))
    }

    nonisolated private static var tokenResponse: (Int, [String: String], Data) {
        (200, [:], Data(#"{"access_token":"renewed","refresh_token":"rotated","expires_in":3600,"token_type":"Bearer"}"#.utf8))
    }

    @Test(arguments: [false, true])
    func cancelledRequestDoesNotReturnOrRefreshTokens(expired: Bool) async throws {
        let api = try client(expired: expired)
        defer { api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        stub { _ in
            Issue.record("キャンセル済みの要求が通信を開始しました。")
            return Self.discoveryResponse
        }
        let request = Task { try await api.auth.accessToken() }
        request.cancel()
        await #expect(throws: CancellationError.self) { try await request.value }
    }

    @Test(arguments: [false, true], [200, 400, 401])
    func cancelledRefreshWaiterDoesNotCancelSharedRefresh(cancelFirst: Bool, status: Int) async throws {
        let api = try client(expired: true)
        defer { api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        try await confirmation("待機者がキャンセルされてもトークン更新を共有する") { refreshed in
            stub { request in
                if request.url?.path == "/.well-known/openid-configuration" {
                    return Self.discoveryResponse
                }
                #expect(request.url?.path == "/token")
                refreshed()
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                return status == 200 ? Self.tokenResponse : (status, [:], Data())
            }
            let first = Task { try await api.auth.accessToken() }
            var startedIterator = started.stream.makeAsyncIterator()
            _ = await startedIterator.next()
            let joined = AsyncStream<Void>.makeStream()
            defer { joined.continuation.finish() }
            let second = Task {
                joined.continuation.yield(())
                return try await api.auth.accessToken()
            }
            var joinedIterator = joined.stream.makeAsyncIterator()
            _ = await joinedIterator.next()
            let cancelled = cancelFirst ? first : second
            let survivor = cancelFirst ? second : first
            cancelled.cancel()
            release.continuation.yield(())
            await #expect(throws: CancellationError.self) { try await cancelled.value }
            if status == 200 {
                let value = try await survivor.value
                #expect(value == "renewed")
                #expect(TokenVault.load(api.settings.storageKey)?.refresh == "rotated")
            } else {
                do {
                    _ = try await survivor.value
                    Issue.record("拒否されたトークン更新が受け入れられました。")
                } catch ZooError.expiredSession { }
                #expect(TokenVault.load(api.settings.storageKey)?.refresh == "test-refresh")
            }
        }
    }

    @Test func logoutInvalidatesAnInFlightRefresh() async throws {
        let api = try client(expired: true)
        defer { api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        stub { request in
            if request.url?.path == "/.well-known/openid-configuration" {
                return Self.discoveryResponse
            }
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return Self.tokenResponse
        }
        let request = Task { try await api.auth.accessToken() }
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        api.auth.logout()
        release.continuation.yield(())
        await #expect(throws: CancellationError.self) { try await request.value }
        #expect(!api.auth.hasSession)
        #expect(TokenVault.load(api.settings.storageKey) == nil)
    }
}
