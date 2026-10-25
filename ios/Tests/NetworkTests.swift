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
        let store = ZooStore(settings: api.settings, transport: api.transport)
        store.me = Person(id: "u", user_name: "user", display_name: "User")
        StubProtocol.state.install { request in
            let body = try JSONDecoder().decode([String: String].self, from: Self.body(request))
            let id = try XCTUnwrap(body["id"])
            if StubProtocol.state.record(id) == 1 { throw URLError(.networkConnectionLost) }
            let response: [String: Any] = ["data": ["id": id, "user_id": "u", "channel_id": "c", "content": "hello", "created_at": "2026-10-26T12:00:00Z"], "txid": "42"]
            return (200, [:], try JSONSerialization.data(withJSONObject: response))
        }
        do { try await store.send("hello", channelID: "c", parentID: nil); XCTFail("Expected a lost response") }
        catch { }
        store.selectedChannel = "c"
        store.receiveChannels([], ready: false)
        try await store.send("hello", channelID: "c", parentID: nil)
        let ids = StubProtocol.state.recordedIDs()
        XCTAssertEqual(ids.count, 2)
        XCTAssertEqual(ids.first, ids.last)
        XCTAssertEqual(store.timeline.count, 1)
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
