import Foundation
import XCTest
@testable import Zoo

private final class StubState: @unchecked Sendable {
    typealias Handler = @Sendable (URLRequest) throws -> (Int, [String: String], Data)
    private let lock = NSLock()
    private var handler: Handler?
    private var ids: [String] = []
    func install(_ value: Handler?) { lock.lock(); defer { lock.unlock() }; handler = value; ids = [] }
    func record(_ id: String) -> Int { lock.lock(); defer { lock.unlock() }; ids.append(id); return ids.count }
    func recordedIDs() -> [String] { lock.lock(); defer { lock.unlock() }; return ids }
    func respond(_ request: URLRequest) throws -> (Int, [String: String], Data) {
        lock.lock(); let callback = handler; lock.unlock()
        guard let callback else { throw URLError(.badServerResponse) }
        return try callback(request)
    }
}

private final class StubProtocol: URLProtocol, @unchecked Sendable {
    static let state = StubState()
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, headers, data) = try Self.state.respond(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: "HTTP/1.1", headerFields: headers)!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

@MainActor
final class NetworkTests: XCTestCase {
    nonisolated private static func body(_ request: URLRequest) throws -> Data {
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

    private func client(expired: Bool = false) throws -> APIClient {
        let settings = ConnectionSettings(server: "https://api.test.invalid", issuer: "https://id.test.invalid",
                                          clientID: UUID().uuidString, resource: "https://api.test.invalid", scope: "openid")
        try TokenVault.save(TokenSet(access: "test-access", refresh: "test-refresh", expires: Date().addingTimeInterval(expired ? -1 : 3600)), account: settings.storageKey)
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StubProtocol.self]
        let transport = HTTPTransport(session: URLSession(configuration: config))
        let auth = Authentication(settings: settings, transport: transport)
        return APIClient(settings: settings, auth: auth, transport: transport)
    }

    func testPushCleanupUsesOriginalServerWithoutBearerAndRejectsSPAResponse() async throws {
        let api = try client()
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        let receipt = PushReceipt(server: "https://previous.test.invalid", account: "old", token: "aabb", environment: "sandbox", secret: String(repeating: "a", count: 64))
        let backend = LivePushBackend(api: api, transport: api.transport)
        StubProtocol.state.install { request in
            XCTAssertEqual(request.url?.host, "previous.test.invalid")
            XCTAssertEqual(request.url?.path, "/api/push/apns/revoke")
            XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
            let body = try JSONSerialization.jsonObject(with: Self.body(request)) as! [String: String]
            XCTAssertEqual(body["id"], receipt.id)
            XCTAssertEqual(body["revocation_secret"], receipt.secret)
            return (200, [:], Data("<html>SPA fallback</html>".utf8))
        }
        do { try await backend.revoke(receipt); XCTFail("A successful HTTP response is not proof of cleanup") }
        catch { XCTAssertTrue(error is DecodingError) }
        StubProtocol.state.install { _ in (200, [:], Data(#"{"ok":true}"#.utf8)) }
        try await backend.revoke(receipt)
    }

    func testPushRegistrationUsesEnvironmentConfigurationAndStableReceipt() async throws {
        let api = try client()
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        let receipt = PushReceipt(server: api.settings.server, account: "user", token: "aabb", environment: "sandbox", secret: String(repeating: "a", count: 64))
        StubProtocol.state.install { request in
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer test-access")
            if request.url?.path == "/api/push/apns/config" {
                return (200, [:], Data(#"{"sandbox":true,"production":false}"#.utf8))
            }
            XCTAssertEqual(request.url?.path, "/api/push/apns/subscriptions")
            let body = try JSONSerialization.jsonObject(with: Self.body(request)) as! [String: Any]
            XCTAssertEqual(body["created_at"] as? Int64, receipt.createdAt)
            XCTAssertEqual(body["environment"] as? String, "sandbox")
            return (200, [:], try JSONSerialization.data(withJSONObject: ["id": receipt.id]))
        }
        try await LivePushBackend(api: api, transport: api.transport).subscribe(receipt)
    }

    func testWritesUseBearerAndDecodeExistingBackendEnvelope() async throws {
        let api = try client()
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        StubProtocol.state.install { request in
            XCTAssertEqual(request.url?.path, "/api/messages")
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer test-access")
            XCTAssertNil(request.value(forHTTPHeaderField: "X-Dev-User"))
            return (200, [:], Data(#"{"data":{"id":"m","user_id":"u","channel_id":"c","content":"hello","created_at":"2026-10-26 10:00:00","parent_id":null,"deleted_at":null,"updated_at":null},"txid":"123"}"#.utf8))
        }
        let message: Message = try await api.write("api/messages", body: ["content": .string("hello"), "channel_id": .string("c")])
        XCTAssertEqual(message.content, "hello")
    }

    func testUnauthorizedIsAnExpiredSession() async throws {
        let api = try client()
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        StubProtocol.state.install { _ in (401, [:], Data()) }
        do { let _: Person = try await api.get("api/users/me"); XCTFail("Unauthorized response accepted") }
        catch ZooError.expiredSession { }
    }

    func testRefreshIsSharedAcrossConcurrentRequests() async throws {
        let api = try client(expired: true)
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        let refreshed = expectation(description: "one token exchange")
        refreshed.assertForOverFulfill = true
        StubProtocol.state.install { request in
            if request.url?.path == "/.well-known/openid-configuration" {
                return (200, [:], Data(#"{"issuer":"https://id.test.invalid","authorization_endpoint":"https://id.test.invalid/authorize","token_endpoint":"https://id.test.invalid/token"}"#.utf8))
            }
            XCTAssertEqual(request.url?.path, "/token")
            refreshed.fulfill()
            return (200, [:], Data(#"{"access_token":"renewed","refresh_token":"rotated","expires_in":3600,"token_type":"Bearer"}"#.utf8))
        }
        async let first = api.auth.accessToken()
        async let second = api.auth.accessToken()
        let values = try await [first, second]
        XCTAssertEqual(values, ["renewed", "renewed"])
        XCTAssertEqual(TokenVault.load(api.settings.storageKey)?.refresh, "rotated")
        let result = await XCTWaiter.fulfillment(of: [refreshed], timeout: 1)
        XCTAssertEqual(result, .completed)
    }

    func testForbiddenShapeClearsRowsBeforeReportingFailure() async throws {
        let api = try client()
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        StubProtocol.state.install { _ in (403, [:], Data(#"{"error":"forbidden"}"#.utf8)) }
        let stream = ShapeStream()
        var events: [String] = []
        await withCheckedContinuation { continuation in
            stream.start(api: api, shape: "messages", channelID: "c", update: { state in
                XCTAssertTrue(state.records.isEmpty)
                events.append("clear")
            }, failure: { _ in
                events.append("failure")
                continuation.resume()
            }, recovered: {})
        }
        stream.stop()
        XCTAssertEqual(events, ["clear", "failure"])
    }

    func testUnknownSubmissionRetainsRetryIDAcrossChannelResync() async throws {
        let api = try client()
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        let session = RepositorySession(api: api)
        let workspace = LiveWorkspaceRepository(session: session)
        workspace.state.me = Person(id: "u", user_name: "user", display_name: "User")
        let repository = LiveMessageRepository(channelID: "c", userID: "u", session: session)
        let store = ConversationViewModel(repository: repository, workspace: workspace, userID: "u")
        StubProtocol.state.install { request in
            let body = try JSONDecoder().decode([String: String].self, from: Self.body(request))
            let id = try XCTUnwrap(body["id"])
            if StubProtocol.state.record(id) == 1 { throw URLError(.networkConnectionLost) }
            let response: [String: Any] = ["data": ["id": id, "user_id": "u", "channel_id": "c", "content": "hello", "created_at": "2026-10-26T12:00:00Z"], "txid": "42"]
            return (200, [:], try JSONSerialization.data(withJSONObject: response))
        }
        do { try await store.send("hello", parentID: nil); XCTFail("Expected a lost response") }
        catch { }
        workspace.state.channels = []
        workspace.state.channelPhase = .resyncing
        store.reconcileChannels()
        try await store.send("hello", parentID: nil)
        let ids = StubProtocol.state.recordedIDs()
        XCTAssertEqual(ids.count, 2)
        XCTAssertEqual(ids.first, ids.last)
        XCTAssertEqual(store.timeline.count, 1)
    }

    func testMessageSubscriptionRecoversOnNoContentAfterTransientFailure() async throws {
        let api = try client()
        let repository = LiveMessageRepository(channelID: "c", userID: "u", session: RepositorySession(api: api))
        defer { repository.stop(); api.auth.logout(); StubProtocol.state.install(nil) }
        StubProtocol.state.install { request in
            if request.url!.path.hasSuffix("message_reactions") {
                return (200, ["electric-offset": "0_0", "electric-handle": "reactions"], Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
            }
            let count = StubProtocol.state.record("messages")
            if count == 1 {
                return (200, ["electric-offset": "0_0", "electric-handle": "messages"], Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
            }
            if count == 2 { return (500, [:], Data()) }
            return (204, [:], Data())
        }
        repository.start()
        await eventually { repository.state.phase == .ready }
        await eventually { repository.state.phase.failure != nil }
        await eventually { repository.state.phase == .ready }
    }

    func testNoContentDoesNotHideAnUndecodableInitialSnapshot() async throws {
        let api = try client()
        let repository = LiveMessageRepository(channelID: "c", userID: "u", session: RepositorySession(api: api))
        defer { repository.stop(); api.auth.logout(); StubProtocol.state.install(nil) }
        let recoveredRequest = expectation(description: "Polled after no-content response")
        StubProtocol.state.install { request in
            if request.url!.path.hasSuffix("message_reactions") {
                return (200, ["electric-offset": "0_0", "electric-handle": "reactions"], Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
            }
            let count = StubProtocol.state.record("messages")
            if count == 1 {
                return (200, ["electric-offset": "0_0", "electric-handle": "messages"], Data(#"[{"key":"broken","value":{"id":"broken"},"headers":{"operation":"insert"}},{"headers":{"control":"up-to-date"}}]"#.utf8))
            }
            if count == 3 { recoveredRequest.fulfill() }
            return (204, [:], Data())
        }
        repository.start()
        await fulfillment(of: [recoveredRequest], timeout: 3)
        XCTAssertNotNil(repository.state.phase.failure)
        XCTAssertTrue(repository.state.timeline.isEmpty)
    }

    func testStoppingLiveConversationReleasesHistoryButKeepsUnreplicatedPosts() throws {
        let api = try client()
        defer { api.auth.logout() }
        let repository = LiveMessageRepository(channelID: "c", userID: "u", session: RepositorySession(api: api))
        let confirmed = Message(id: "old", user_id: "u", channel_id: "c", content: "old", created_at: ZooDate.now())
        let outgoing = Message(id: "new", user_id: "u", channel_id: "c", content: "new", created_at: ZooDate.now())
        repository.state.receive([confirmed])
        repository.state.acknowledge(outgoing)
        repository.state.reactions = [Reaction(id: "r", message_id: "old", user_id: "u", emoji: "👍")]
        repository.stop()
        XCTAssertEqual(repository.state.timeline.map(\.id), ["new"])
        XCTAssertTrue(repository.state.reactions.isEmpty)
        repository.state.receive([confirmed, outgoing])
        XCTAssertEqual(repository.state.timeline.count, 2)
        repository.stop()
        XCTAssertTrue(repository.state.timeline.isEmpty)
    }

    func testInvalidatedSessionRejectsLateWriteResponse() async throws {
        let api = try client()
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        let session = RepositorySession(api: api)
        let started = expectation(description: "Request received")
        let release = DispatchSemaphore(value: 0)
        defer { release.signal() }
        StubProtocol.state.install { _ in
            started.fulfill()
            guard release.wait(timeout: .now() + 3) == .success else { throw URLError(.timedOut) }
            return (200, [:], Data(#"{"data":{"ok":true},"txid":"1"}"#.utf8))
        }
        let write = Task { () throws -> JSONValue in try await session.write("api/messages") }
        await fulfillment(of: [started], timeout: 2)
        session.invalidate()
        release.signal()
        do { _ = try await write.value; XCTFail("Accepted a response from the previous session") }
        catch { XCTAssertTrue(error is CancellationError) }
    }

    private func eventually(_ condition: @MainActor () -> Bool, file: StaticString = #filePath, line: UInt = #line) async {
        let deadline = ContinuousClock.now + .seconds(3)
        while !condition(), ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(condition(), file: file, line: line)
    }

    func testShapePublishesOnlyAfterAllInitialPages() async throws {
        let api = try client()
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        StubProtocol.state.install { request in
            let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)!.queryItems!
            let offset = query.first { $0.name == "offset" }?.value ?? ""
            XCTAssertEqual(query.first { $0.name == "channel_id" }?.value, "c")
            if offset == "-1" {
                return (200, ["electric-offset": "0_0", "electric-handle": "shape-1"], Data(#"[{"key":"one","value":{"id":"one","user_name":"one","display_name":"One","is_bot":"false"},"headers":{"operation":"insert"}}]"#.utf8))
            }
            XCTAssertEqual(offset, "0_0")
            XCTAssertEqual(query.first { $0.name == "handle" }?.value, "shape-1")
            XCTAssertNil(query.first { $0.name == "live" })
            return (200, ["electric-offset": "1_0", "electric-handle": "shape-1"], Data(#"[{"key":"two","value":{"id":"two","user_name":"two","display_name":"Two","is_bot":"false"},"headers":{"operation":"insert"}},{"headers":{"control":"up-to-date"}}]"#.utf8))
        }
        let stream = ShapeStream()
        let rows: [Person] = try await withCheckedThrowingContinuation { continuation in
            stream.start(api: api, shape: "users", channelID: "c", update: { state in
                XCTAssertTrue(state.ready)
                continuation.resume(returning: try state.decode(Person.self))
                stream.stop()
            }, failure: { error in
                continuation.resume(throwing: error)
                stream.stop()
            }, recovered: {})
        }
        XCTAssertEqual(Set(rows.map(\.id)), ["one", "two"])
    }
}
