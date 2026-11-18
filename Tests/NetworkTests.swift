import Foundation
import Security
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
@Suite("通話中の認証情報保存")
struct TokenVaultTests {
    @Test("新規保存と旧設定からの移行で、ロック中に更新できる端末専用項目を使う", arguments: [false, true])
    func backgroundAccessibility(legacy: Bool) throws {
        let account = "test-background-call-\(UUID().uuidString)"
        defer { TokenVault.delete(account) }
        let token = TokenSet(access: "test-access", refresh: "test-refresh", expires: .now.addingTimeInterval(3600))
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                    kSecAttrService as String: "works.zli.zoo.session", kSecAttrAccount as String: account]
        if legacy {
            var item = query
            item[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            item[kSecValueData as String] = try JSONEncoder().encode(token)
            #expect(SecItemAdd(item as CFDictionary, nil) == errSecSuccess)
        } else { try TokenVault.save(token, account: account) }
        #expect(try TokenVault.load(account)?.refresh == "test-refresh")

        var attributesQuery = query
        attributesQuery[kSecReturnAttributes as String] = true
        var result: CFTypeRef?
        #expect(SecItemCopyMatching(attributesQuery as CFDictionary, &result) == errSecSuccess)
        let attributes = try #require(result as? [String: Any])
        #expect(attributes[kSecAttrAccessible as String] as? String == kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)

        try TokenVault.save(TokenSet(access: "test-new-access", refresh: "test-rotated-refresh", expires: .now.addingTimeInterval(3600)), account: account)
        let reloaded = try #require(try TokenVault.load(account))
        #expect(reloaded.access == "test-new-access" && reloaded.refresh == "test-rotated-refresh")
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

    @Test("自分の登録した端末に一度だけ送り、端末への到着だけを受信済みにする")
    func testPushDeliveryAcknowledgement() async throws {
        let api = try client()
        let user = UUID().uuidString
        let account = PushReceipt.digest(api.settings.storageKey + "|" + user)
        let preference = "zoo.push.enabled." + account
        UserDefaults.standard.set(true, forKey: preference)
        let receipts = PreviewPushReceiptStore()
        let previewKeys = InMemoryNotificationPreviewKeys()
        let controller = NotificationController(transport: api.transport, receiptStore: receipts, previewKeys: previewKeys)
        let started = AsyncStream<UUID>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer {
            started.continuation.finish()
            release.continuation.finish()
            UserDefaults.standard.removeObject(forKey: preference)
            UserDefaults.standard.removeObject(forKey: "zoo.push.test." + account)
            api.auth.logout()
            api.transport.session.invalidateAndCancel()
            stub(nil)
        }
        let requests = Mutex(0)
        let id = PushReceipt.digest("sandbox:aabb")
        stub { request in
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            switch request.url?.path {
            case "/api/push/apns/config": return (200, [:], Data(#"{"sandbox":true,"production":false}"#.utf8))
            case "/api/push/apns/subscriptions": return (200, [:], try JSONEncoder().encode(["id": id]))
            default:
                #expect(request.url?.path == "/api/push/apns/test")
                #expect(request.httpMethod == "POST")
                let body = try JSONDecoder().decode([String: String].self, from: StubProtocol.body(request))
                #expect(body["id"] == id)
                #expect(body["revocation_secret"]?.count == 64)
                #expect(body.count == 3)
                let testID = try #require(body["test_id"].flatMap(UUID.init(uuidString:)))
                requests.withLock { $0 += 1 }
                started.continuation.yield(testID)
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                return (200, [:], Data(#"{"ok":true}"#.utf8))
            }
        }
        controller.connect(api: api, userID: user)
        #expect(previewKeys.load() == nil)
        await controller.sendTest()
        #expect(!controller.canSendTest)
        await controller.received(token: Data([0xaa, 0xbb]))?.value
        #expect(controller.canSendTest)
        let receipt = try #require(try receipts.load())
        let previewKey = NotificationPreviewKey(userID: user, secret: receipt.secret)
        #expect(previewKeys.load() == previewKey)
        await #expect(throws: ZooError.self) {
            try await LivePushBackend(api: api, transport: api.transport).sendTest(to: receipt, account: "別のアカウント", id: UUID())
        }
        let send = Task { await controller.sendTest() }
        var iterator = started.stream.makeAsyncIterator()
        let testID = try #require(await iterator.next())
        await controller.sendTest()
        #expect(requests.withLock { $0 } == 1)
        #expect(!controller.receive(.test(userID: UUID().uuidString, id: testID)))
        #expect(controller.receive(.test(userID: user, id: UUID())))
        #expect(controller.testStatus == nil)
        #expect(controller.receive(.test(userID: user, id: testID)))
        release.continuation.yield(())
        await send.value
        #expect(controller.testStatus == "このiPhoneでテスト通知を受信しました。")

        let second = Task { await controller.sendTest() }
        let nextID = try #require(await iterator.next())
        #expect(nextID != testID)
        release.continuation.yield(())
        await second.value
        #expect(controller.testStatus == "テスト通知を送信しました。")
        let restarted = NotificationController(transport: api.transport, receiptStore: receipts, previewKeys: previewKeys)
        restarted.opened(.test(userID: user, id: nextID))
        restarted.connect(api: api, userID: user)
        #expect(previewKeys.load() == previewKey)
        #expect(restarted.destination == nil)
        #expect(restarted.testStatus == "このiPhoneでテスト通知を受信しました。")

        stub { _ in
            requests.withLock { $0 += 1 }
            return (503, [:], Data())
        }
        await controller.sendTest()
        #expect(requests.withLock { $0 } == 3)
        #expect(controller.receive(.test(userID: user, id: nextID)))
        #expect(controller.testStatus?.contains("送信できませんでした") == true)
        restarted.connect(api: api, userID: UUID().uuidString)
        #expect(previewKeys.load() == nil)
    }

    @Test("トークン変更で以前の鍵を即時削除し、新しい登録が成功するまで鍵を共有しない")
    func rotatedTokenWaitsForSuccessfulRegistration() async throws {
        let api = try client()
        let user = UUID().uuidString
        let preference = "zoo.push.enabled." + PushReceipt.digest(api.settings.storageKey + "|" + user)
        UserDefaults.standard.set(true, forKey: preference)
        let receipts = PreviewPushReceiptStore(), keys = InMemoryNotificationPreviewKeys()
        let controller = NotificationController(transport: api.transport, receiptStore: receipts, previewKeys: keys)
        defer {
            UserDefaults.standard.removeObject(forKey: preference)
            api.auth.logout()
            api.transport.session.invalidateAndCancel()
            stub(nil)
        }
        controller.connect(api: api, userID: user)
        let failRegistration = Mutex(false)
        stub { request in
            switch request.url?.path {
            case "/api/push/apns/config":
                return (200, [:], Data(#"{"sandbox":true,"production":false}"#.utf8))
            case "/api/push/apns/subscriptions":
                if failRegistration.withLock({ $0 }) { return (503, [:], Data()) }
                let body = try #require(JSONSerialization.jsonObject(with: StubProtocol.body(request)) as? [String: Any])
                let token = try #require(body["device_token"] as? String)
                let id = PushReceipt.digest("sandbox:" + token)
                return (200, [:], try JSONEncoder().encode(["id": id]))
            default:
                #expect(request.url?.path == "/api/push/apns/revoke")
                return (200, [:], Data(#"{"ok":true}"#.utf8))
            }
        }
        await controller.received(token: Data([0xaa]))?.value
        let previous = try #require(keys.load())
        failRegistration.withLock { $0 = true }
        let rotation = controller.received(token: Data([0xbb]))
        #expect(keys.load() == nil)
        await rotation?.value
        #expect(keys.load() == nil)
        let retry = NotificationController(transport: api.transport, receiptStore: receipts, previewKeys: keys)
        retry.connect(api: api, userID: user)
        #expect(keys.load() == nil)
        failRegistration.withLock { $0 = false }
        await retry.received(token: Data([0xbb]))?.value
        let replacement = try #require(keys.load())
        #expect(replacement.userID == user)
        #expect(replacement != previous)
        await retry.resume(authorization: { .denied })
        #expect(keys.load() == nil)
        await retry.received(token: Data([0xbb]))?.value
        #expect(keys.load() == nil)
    }

    @Test("登録応答の待機中にログアウトすると、オフラインで解除に失敗しても旧利用者の鍵を復元しない")
    func logoutInvalidatesPendingPreviewRegistration() async throws {
        let api = try client()
        let user = UUID().uuidString
        let preference = "zoo.push.enabled." + PushReceipt.digest(api.settings.storageKey + "|" + user)
        UserDefaults.standard.set(true, forKey: preference)
        let receipts = PreviewPushReceiptStore(), keys = InMemoryNotificationPreviewKeys()
        let controller = NotificationController(transport: api.transport, receiptStore: receipts, previewKeys: keys)
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer {
            started.continuation.finish()
            release.continuation.finish()
            UserDefaults.standard.removeObject(forKey: preference)
            api.auth.logout()
            api.transport.session.invalidateAndCancel()
            stub(nil)
        }
        let id = PushReceipt.digest("sandbox:aa")
        let waitForResponse = Mutex(false)
        stub { request in
            switch request.url?.path {
            case "/api/push/apns/config":
                return (200, [:], Data(#"{"sandbox":true,"production":false}"#.utf8))
            case "/api/push/apns/subscriptions":
                if waitForResponse.withLock({ $0 }) {
                    started.continuation.yield(())
                    var iterator = release.stream.makeAsyncIterator()
                    _ = await iterator.next()
                }
                return (200, [:], try JSONEncoder().encode(["id": id]))
            default:
                #expect(request.url?.path == "/api/push/apns/revoke")
                throw URLError(.notConnectedToInternet)
            }
        }
        controller.connect(api: api, userID: user)
        await controller.received(token: Data([0xaa]))?.value
        #expect(keys.load() != nil)
        waitForResponse.withLock { $0 = true }
        let renewal = controller.received(token: Data([0xaa]))
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        controller.logout()
        #expect(keys.load() == nil)
        release.continuation.yield(())
        await renewal?.value
        #expect(keys.load() == nil)
        await controller.revokeRegistration().value
        #expect(try receipts.load()?.revocationPending == true)
        #expect(keys.load() == nil)
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

    @Test("認証復元前やバックグラウンドで届いた本人のテスト通知を復帰後に照合する")
    func testPendingAndDeliveredNoticesOnResume() async throws {
        let api = try client()
        let user = UUID().uuidString
        let id = UUID()
        let account = PushReceipt.digest(api.settings.storageKey + "|" + user)
        let preference = "zoo.push.enabled." + account
        let attemptKey = "zoo.push.test." + account
        let pending = try JSONSerialization.data(withJSONObject: ["id": id.uuidString, "received": false])
        UserDefaults.standard.set(true, forKey: preference)
        defer {
            UserDefaults.standard.removeObject(forKey: preference)
            UserDefaults.standard.removeObject(forKey: attemptKey)
            api.auth.logout()
            api.transport.session.invalidateAndCancel()
        }
        let matching = PushNotice.test(userID: user, id: id)
        let unrelated = PushNotice.test(userID: UUID().uuidString, id: id)
        let controller = NotificationController(transport: api.transport, receiptStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys())
        let delegate = NotificationDelegate()
        UserDefaults.standard.set(pending, forKey: attemptKey)
        #expect(delegate.presentationOptions(for: unrelated).isEmpty)
        #expect(delegate.presentationOptions(for: matching).isEmpty)
        delegate.bind(controller)
        #expect(controller.testStatus == nil)
        controller.connect(api: api, userID: user)
        #expect(controller.testStatus == "このiPhoneでテスト通知を受信しました。")

        UserDefaults.standard.set(pending, forKey: attemptKey)
        let restored = NotificationController(transport: api.transport, receiptStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys())
        delegate.bind(restored)
        #expect(delegate.presentationOptions(for: matching).isEmpty)
        #expect(delegate.presentationOptions(for: unrelated).isEmpty)
        restored.connect(api: api, userID: user)
        #expect(restored.testStatus == nil)
        await restored.checkDeliveredTest { [unrelated, .test(userID: user, id: UUID())] }
        #expect(restored.testStatus == nil)
        await restored.checkDeliveredTest { [unrelated, matching] }
        #expect(restored.testStatus == "このiPhoneでテスト通知を受信しました。")
        await restored.checkDeliveredTest {
            Issue.record("受信確認済みのテストで配信済み通知を再取得しました。")
            return []
        }
        UserDefaults.standard.removeObject(forKey: attemptKey)
        restored.connect(api: api, userID: user)
        await restored.checkDeliveredTest {
            Issue.record("テストを送っていないアカウントで配信済み通知を取得しました。")
            return []
        }
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
                #expect(try TokenVault.load(api.settings.storageKey)?.refresh == "rotated")
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
                #expect(try TokenVault.load(api.settings.storageKey)?.refresh == "rotated")
            } else {
                do {
                    _ = try await survivor.value
                    Issue.record("拒否されたトークン更新が受け入れられました。")
                } catch ZooError.expiredSession { }
                #expect(try TokenVault.load(api.settings.storageKey)?.refresh == "test-refresh")
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
        #expect(try !api.auth.hasSession)
        #expect(try TokenVault.load(api.settings.storageKey) == nil)
    }
}
