import XCTest
@testable import Zoo

@MainActor
private final class MemoryPushStore: PushReceiptStore {
    var receipt: PushReceipt?
    var failSave = false
    func load() throws -> PushReceipt? { receipt }
    func save(_ receipt: PushReceipt?) throws {
        if failSave { throw URLError(.cannotWriteToFile) }
        self.receipt = receipt
    }
}

@MainActor
private final class StubPushBackend: PushBackend {
    let server = "https://zli.chat"
    var subscribed: [PushReceipt] = []
    var revoked: [PushReceipt] = []
    var failSubscribe = false
    var failRevoke = false
    var beforeSubscribe: (() async -> Void)?
    func subscribe(_ receipt: PushReceipt) async throws {
        subscribed.append(receipt)
        await beforeSubscribe?()
        if failSubscribe { throw URLError(.networkConnectionLost) }
    }
    func revoke(_ receipt: PushReceipt) async throws {
        revoked.append(receipt)
        if failRevoke { throw URLError(.notConnectedToInternet) }
    }
}

@MainActor
final class PushTests: XCTestCase {
    func testLostResponseRetriesSameCapability() async {
        let store = MemoryPushStore(), backend = StubPushBackend()
        let registration = PushRegistration(store: store)
        backend.failSubscribe = true
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        XCTAssertNotNil(store.receipt)
        backend.failSubscribe = false
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        XCTAssertEqual(backend.subscribed.count, 2)
        XCTAssertEqual(backend.subscribed.first, backend.subscribed.last)
        XCTAssertTrue(backend.revoked.isEmpty)
    }

    func testFailedRevocationPreventsAccountTakeoverAndRetriesAfterRestart() async {
        let store = MemoryPushStore(), backend = StubPushBackend()
        let registration = PushRegistration(store: store)
        await registration.update(backend: backend, account: "first", token: "aabb", environment: "sandbox").value
        let original = store.receipt
        backend.failRevoke = true
        await registration.update(backend: backend, account: nil, token: nil, environment: "sandbox").value
        await registration.update(backend: backend, account: "second", token: "aabb", environment: "sandbox").value
        XCTAssertEqual(store.receipt, original)
        XCTAssertEqual(backend.subscribed.count, 1)
        backend.failRevoke = false
        let restarted = PushRegistration(store: store)
        await restarted.update(backend: backend, account: "second", token: "aabb", environment: "sandbox").value
        XCTAssertEqual(backend.subscribed.count, 2)
        XCTAssertNotEqual(store.receipt?.secret, original?.secret)
        XCTAssertEqual(store.receipt?.account, "second")
    }

    func testLogoutWaitsForInFlightRegistrationThenRevokes() async {
        let store = MemoryPushStore(), backend = StubPushBackend()
        let registration = PushRegistration(store: store)
        let started = expectation(description: "Registration began")
        var release: CheckedContinuation<Void, Never>?
        backend.beforeSubscribe = {
            await withCheckedContinuation { release = $0; started.fulfill() }
        }
        registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox")
        await fulfillment(of: [started], timeout: 2)
        let logout = registration.update(backend: backend, account: nil, token: nil, environment: "sandbox")
        XCTAssertTrue(backend.revoked.isEmpty)
        release?.resume()
        await logout.value
        XCTAssertNil(store.receipt)
        XCTAssertEqual(backend.revoked, backend.subscribed)
    }

    func testKeychainFailureNeverSendsUnrecoverableRegistration() async {
        let store = MemoryPushStore(), backend = StubPushBackend()
        store.failSave = true
        await PushRegistration(store: store).update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        XCTAssertTrue(backend.subscribed.isEmpty)
    }

    func testExpiredReceiptRotatesOnlyAfterSuccessfulRevocation() async {
        let store = MemoryPushStore(), backend = StubPushBackend()
        let registration = PushRegistration(store: store)
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        store.receipt?.createdAt = Int64(Date().timeIntervalSince1970) - 601
        let old = store.receipt
        backend.failRevoke = true
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        XCTAssertEqual(store.receipt, old)
        XCTAssertEqual(backend.subscribed.count, 1)
        backend.failRevoke = false
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        XCTAssertNotEqual(store.receipt?.secret, old?.secret)
        XCTAssertGreaterThan(store.receipt!.createdAt, old!.createdAt)
    }

    func testTokenRotationRevokesPreviousDeviceBeforeRegistering() async {
        let store = MemoryPushStore(), backend = StubPushBackend()
        let registration = PushRegistration(store: store)
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        await registration.update(backend: backend, account: "user", token: "ccdd", environment: "sandbox").value
        XCTAssertEqual(backend.revoked.first?.token, "aabb")
        XCTAssertEqual(store.receipt?.token, "ccdd")
    }

    func testNotificationPayloadRejectsUntrustedNavigation() {
        let user = UUID().uuidString, channel = UUID().uuidString
        XCTAssertNotNil(PushDestination(payload: ["user_id": user, "channel_id": channel]))
        XCTAssertNil(PushDestination(payload: ["user_id": user, "channel_id": "../private"]))
        XCTAssertNil(PushDestination(payload: ["channel_id": channel]))
        XCTAssertNil(PushDestination(payload: ["user_id": user, "channel_id": channel, "message_id": "https://evil.example"]))
    }

    func testHTTPSCallbackRequiresExactOriginAndIssuer() throws {
        let base = "https://zli.chat/ios/oauth/callback?state=s&code=ok"
        let issuer = "https://account.zli.works/api/auth"
        XCTAssertEqual(try PKCE.code(from: URL(string: base + "&iss=" + issuer)!, expectedState: "s", expectedIssuer: issuer), "ok")
        for value in [base, base + "&iss=https://evil.example", base + "&iss=" + issuer + "&iss=" + issuer,
                      base.replacingOccurrences(of: "zli.chat", with: "user@zli.chat"),
                      base.replacingOccurrences(of: "zli.chat", with: "zli.chat:443"), base + "#fragment"] {
            XCTAssertThrowsError(try PKCE.code(from: URL(string: value)!, expectedState: "s", expectedIssuer: issuer))
        }
    }
}
