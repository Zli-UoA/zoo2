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
            return (200, [:], Data("<html>SPA のフォールバックページ</html>".utf8))
        }
        do { try await backend.revoke(receipt); XCTFail("HTTP 応答の成功だけでは登録解除の完了とみなせません。") }
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
            return (200, [:], Data(#"{"data":{"id":"m","user_id":"u","channel_id":"c","content":"こんにちは","created_at":"2026-10-26 10:00:00","parent_id":null,"deleted_at":null,"updated_at":null},"txid":"123"}"#.utf8))
        }
        let message: Message = try await api.write("api/messages", body: ["content": .string("こんにちは"), "channel_id": .string("c")])
        XCTAssertEqual(message.content, "こんにちは")
    }

    func testUnauthorizedIsAnExpiredSession() async throws {
        let api = try client()
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        StubProtocol.state.install { _ in (401, [:], Data()) }
        do { let _: Person = try await api.get("api/users/me"); XCTFail("認証エラーの応答が受け入れられました。") }
        catch ZooError.expiredSession { }
    }

    func testRefreshIsSharedAcrossConcurrentRequests() async throws {
        let api = try client(expired: true)
        defer { api.auth.logout(); StubProtocol.state.install(nil) }
        let refreshed = expectation(description: "トークン交換が 1 回だけ行われる")
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


}
