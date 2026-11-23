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
    var subscribing = false
    var revokedDuringSubscribe = false
    var failSubscribe = false
    var failRevoke = false
    var beforeSubscribe: (() async -> Void)?
    var beforeRevoke: (() async -> Void)?
    func subscribe(_ receipt: PushReceipt) async throws {
        subscribed.append(receipt)
        subscribing = true
        defer { subscribing = false }
        await beforeSubscribe?()
        if failSubscribe { throw URLError(.networkConnectionLost) }
    }
    func revoke(_ receipt: PushReceipt) async throws {
        revokedDuringSubscribe = revokedDuringSubscribe || subscribing
        revoked.append(receipt)
        await beforeRevoke?()
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
        XCTAssertEqual(store.receipt?.id, original?.id)
        XCTAssertEqual(store.receipt?.secret, original?.secret)
        XCTAssertEqual(store.receipt?.revocationPending, true)
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
        let started = expectation(description: "通知の登録が開始された")
        let release = AsyncStream<Void>.makeStream()
        defer { release.continuation.finish() }
        backend.beforeSubscribe = {
            started.fulfill()
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
        }
        registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox")
        await fulfillment(of: [started], timeout: 2)
        let logout = registration.update(backend: backend, account: nil, token: nil, environment: "sandbox")
        XCTAssertTrue(backend.revoked.isEmpty)
        release.continuation.finish()
        await logout.value
        XCTAssertNil(store.receipt)
        XCTAssertFalse(backend.revokedDuringSubscribe)
        XCTAssertEqual(backend.revoked.map(\.secret), backend.subscribed.map(\.secret))
    }

    func testKeychainFailureNeverSendsUnrecoverableRegistration() async {
        let store = MemoryPushStore(), backend = StubPushBackend()
        store.failSave = true
        await PushRegistration(store: store).update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        XCTAssertTrue(backend.subscribed.isEmpty)
    }

    func testRenewalKeepsSubscriptionWhenOfflineOrClockChanges() async throws {
        let store = MemoryPushStore(), backend = StubPushBackend()
        let registration = PushRegistration(store: store)
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        let original = try XCTUnwrap(store.receipt)
        for offset: Int64 in [-601, 300] {
            store.receipt?.createdAt = original.createdAt + offset
            backend.failSubscribe = true
            await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
            let renewed = try XCTUnwrap(store.receipt)
            XCTAssertEqual(renewed.id, original.id)
            XCTAssertEqual(renewed.secret, original.secret)
            XCTAssertNil(renewed.revocationPending)
            XCTAssertNotEqual(renewed.createdAt, original.createdAt + offset)
            XCTAssertEqual(backend.subscribed.last, renewed)
            XCTAssertTrue(backend.revoked.isEmpty)
            backend.failSubscribe = false
            let attempts = backend.subscribed.count
            await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
            XCTAssertEqual(backend.subscribed.count, attempts + 1)
            XCTAssertEqual(backend.subscribed.last, renewed)
        }
    }

    func testReenablingAfterLostRevocationFinishesRevocationFirst() async throws {
        let store = MemoryPushStore(), backend = StubPushBackend()
        let registration = PushRegistration(store: store)
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        let original = try XCTUnwrap(store.receipt)
        backend.failRevoke = true
        await registration.update(backend: backend, account: nil, token: nil, environment: "sandbox").value
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        XCTAssertEqual(store.receipt?.revocationPending, true)
        XCTAssertEqual(backend.subscribed, [original])
        backend.failRevoke = false
        let restarted = PushRegistration(store: store)
        await restarted.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        XCTAssertEqual(backend.revoked.last?.secret, original.secret)
        XCTAssertEqual(store.receipt?.id, original.id)
        XCTAssertNotEqual(store.receipt?.secret, original.secret)
        XCTAssertNil(store.receipt?.revocationPending)
    }

    func testTokenRotationRevokesPreviousDeviceBeforeRegistering() async throws {
        let store = MemoryPushStore(), backend = StubPushBackend()
        let registration = PushRegistration(store: store)
        await registration.update(backend: backend, account: "user", token: "aabb", environment: "sandbox").value
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        backend.beforeRevoke = {
            started.continuation.yield(())
            for await _ in release.stream { break }
        }
        let update = registration.update(backend: backend, account: "user", token: "ccdd", environment: "sandbox")
        let completion = Task { await update.value; started.continuation.finish() }
        defer { completion.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        let received = await iterator.next()
        XCTAssertNotNil(received)
        XCTAssertEqual(backend.subscribed.map(\.token), ["aabb"])
        release.continuation.finish()
        await update.value
        XCTAssertEqual(backend.subscribed.map(\.token), ["aabb", "ccdd"])
        XCTAssertEqual(backend.revoked.first?.token, "aabb")
        XCTAssertEqual(store.receipt?.token, "ccdd")
    }

    func testNotificationPayloadRejectsUntrustedNavigation() {
        let user = UUID().uuidString, channel = UUID().uuidString
        XCTAssertNotNil(PushDestination(payload: ["user_id": user, "channel_id": channel]))
        XCTAssertNil(PushDestination(payload: ["user_id": user, "channel_id": "../private"]))
        XCTAssertNil(PushDestination(payload: ["channel_id": channel]))
        XCTAssertNil(PushDestination(payload: ["user_id": user, "channel_id": channel, "message_id": "https://evil.example"]))
        let id = UUID()
        let test: [String: String] = ["type": "push_test", "user_id": user, "test_id": id.uuidString, "channel_id": channel]
        XCTAssertEqual(PushNotice(payload: test), .test(userID: user, id: id))
        XCTAssertNil(PushDestination(payload: test))
        XCTAssertNil(PushNotice(payload: ["type": "push_test", "user_id": user]))
    }

}
