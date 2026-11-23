import Foundation
import Security
import Synchronization
import Testing
import UserNotifications
@testable import Zoo

@MainActor
private func awaitCallback<Value: Sendable>(
    _ start: (@escaping @Sendable (Result<Value, any Error>) -> Void) -> Void
) async throws -> Value {
    let state = Mutex((count: 0, result: Optional<Result<Value, any Error>>.none))
    start { result in
        let count = state.withLock {
            $0.count += 1
            if $0.result == nil { $0.result = result }
            return $0.count
        }
        #expect(count == 1, "完了コールバックが複数回呼ばれました。")
    }
    let deadline = ContinuousClock.now.advanced(by: .seconds(10))
    while state.withLock({ $0.result == nil }), ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(1))
    }
    return try #require(state.withLock { $0.result }, "完了コールバックが期限内に呼ばれませんでした。").get()
}

// 公開の復元APIで通知を作り、OSが呼ぶObjective-Cのデリゲート入口まで検証する。
private final class NotificationTestCoder: NSCoder {
    private let values: [String: Any]
    init(_ values: [String: Any]) { self.values = values }
    override var allowsKeyedCoding: Bool { true }
    override func decodeObject(forKey key: String) -> Any? { values[key] }

    static func notification(payload: [String: String]) throws -> UNNotification {
        let content = UNMutableNotificationContent()
        content.userInfo = payload
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        return try #require(UNNotification(coder: NotificationTestCoder(["request": request, "date": Date()])))
    }
}

// URLProtocolの継承にはunchecked適合が必要なため、追加する可変状態はすべてMutexで保護します。
final class StubProtocol: URLProtocol, @unchecked Sendable {
    typealias Handler = @Sendable (URLRequest) async throws -> (Int, [String: String], Data)
    private static let handlers = Mutex<[String: Handler]>([:])
    private let callbackLock = NSRecursiveLock()
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
                deliver { client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed) }
                deliver { client?.urlProtocol(self, didLoad: data) }
                deliver { client?.urlProtocolDidFinishLoading(self) }
            } catch {
                guard !Task.isCancelled else { return }
                deliver { client?.urlProtocol(self, didFailWithError: error) }
            }
        }
        let stopped = loading.withLock { state in
            if !state.stopped { state.task = task }
            return state.stopped
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
        let task = callbackLock.withLock { loading.withLock { state in
            state.stopped = true
            defer { state.task = nil }
            return state.task
        } }
        task?.cancel()
    }
}

@MainActor
@Suite("通話中の認証情報保存")
struct TokenVaultTests {
    @Test("認証情報の削除後に記録削除だけが失敗した場合は案内を表示し、復旧後に再ログインできる")
    func failedMarkerRemovalReportsRecovery() throws {
        let account = "marker-test-" + UUID().uuidString
        defer { try? TokenVault.delete(account) }
        let token = TokenSet(access: "test-access", refresh: nil, expires: .now.addingTimeInterval(3600))
        try TokenVault.save(token, account: account)
        do {
            try TokenVault.delete(account, removeMarker: { _ in throw CocoaError(.fileWriteNoPermission) })
            Issue.record("ログアウト記録の削除失敗を通知しませんでした。")
        } catch ZooError.auth(let message) {
            #expect(message.contains("認証情報は削除しましたが"))
            #expect(message.contains("再インストールしてください"))
            #expect(message.contains("下書きも削除されます"))
        }
        var deletionStatus: OSStatus?
        #expect(try TokenVault.load(account, deleteItem: { query in
            let status = SecItemDelete(query)
            deletionStatus = status
            return status
        }) == nil)
        #expect(deletionStatus == errSecItemNotFound)
        try TokenVault.save(token, account: account)
        #expect(try TokenVault.load(account)?.access == token.access)
    }

    @Test("Keychain削除失敗後は旧トークンを復元せず、削除を再試行する")
    func failedLogoutIsRemembered() throws {
        let account = "logout-test-" + UUID().uuidString
        defer { try? TokenVault.delete(account) }
        try TokenVault.save(TokenSet(access: "test-access", refresh: "test-refresh", expires: .now.addingTimeInterval(3600)), account: account)
        #expect(throws: ZooError.self) {
            try TokenVault.delete(account, deleteItem: { _ in errSecInteractionNotAllowed })
        }
        var retries = 0
        #expect(throws: ZooError.self) {
            _ = try TokenVault.load(account, deleteItem: { _ in retries += 1; return errSecInteractionNotAllowed })
        }
        #expect(retries == 1)
        #expect(try TokenVault.load(account) == nil)
        #expect(try TokenVault.load(account) == nil)
        try TokenVault.save(TokenSet(access: "new-test-access", refresh: nil, expires: .now.addingTimeInterval(3600)), account: account)
        #expect(try TokenVault.load(account)?.access == "new-test-access")
    }

    @Test("新規保存と旧設定からの移行で、ロック中に更新できる端末専用項目を使う", arguments: [false, true])
    func backgroundAccessibility(legacy: Bool) throws {
        let account = "test-background-call-\(UUID().uuidString)"
        defer { try? TokenVault.delete(account) }
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
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
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
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
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
        let started = AsyncStream<UUID?>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer {
            started.continuation.finish()
            release.continuation.finish()
            UserDefaults.standard.removeObject(forKey: preference)
            UserDefaults.standard.removeObject(forKey: "zoo.push.test." + account)
            try? api.auth.logout()
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
        let send = Task {
            defer { started.continuation.yield(nil) }
            await controller.sendTest()
        }
        var iterator = started.stream.makeAsyncIterator()
        let firstEvent = try #require(await iterator.next())
        let testID = try #require(firstEvent)
        await controller.sendTest()
        #expect(requests.withLock { $0 } == 1)
        #expect(!controller.receive(.test(userID: UUID().uuidString, id: testID)))
        #expect(controller.receive(.test(userID: user, id: UUID())))
        #expect(controller.testStatus == nil)
        #expect(controller.receive(.test(userID: user, id: testID)))
        release.continuation.yield(())
        await send.value
        #expect(await iterator.next() == .some(nil))
        #expect(controller.testStatus == "この端末でテスト通知を受信しました。")

        let second = Task {
            defer { started.continuation.yield(nil) }
            await controller.sendTest()
        }
        let nextEvent = try #require(await iterator.next())
        let nextID = try #require(nextEvent)
        #expect(nextID != testID)
        release.continuation.yield(())
        await second.value
        #expect(await iterator.next() == .some(nil))
        #expect(controller.testStatus == "テスト通知を送信しました。")
        let restarted = NotificationController(transport: api.transport, receiptStore: receipts, previewKeys: previewKeys)
        restarted.opened(.test(userID: user, id: nextID))
        restarted.connect(api: api, userID: user)
        #expect(previewKeys.load() == previewKey)
        #expect(restarted.destination == nil)
        #expect(restarted.testStatus == "この端末でテスト通知を受信しました。")

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
            try? api.auth.logout()
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
            try? api.auth.logout()
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
        let renewal = try #require(controller.received(token: Data([0xaa])))
        let completion = Task { await renewal.value; started.continuation.finish() }
        defer { renewal.cancel(); completion.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        controller.logout()
        #expect(keys.load() == nil)
        release.continuation.yield(())
        await renewal.value
        #expect(keys.load() == nil)
        await controller.revokeRegistration().value
        #expect(try receipts.load()?.revocationPending == true)
        #expect(keys.load() == nil)
    }

    @Test func testWritesUseBearerAndDecodeExistingBackendEnvelope() async throws {
        let api = try client()
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
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
            try? api.auth.logout()
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
        #expect(controller.testStatus == "この端末でテスト通知を受信しました。")

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
        #expect(restored.testStatus == "この端末でテスト通知を受信しました。")
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

    @Test("通知表示の完了をメインスレッドで一度だけ返す", arguments: [true, false])
    func testForegroundNotificationCompletion(validPayload: Bool) async throws {
        let api = try client()
        let user = UUID().uuidString
        let preference = "zoo.push.enabled." + PushReceipt.digest(api.settings.storageKey + "|" + user)
        UserDefaults.standard.set(true, forKey: preference)
        defer {
            UserDefaults.standard.removeObject(forKey: preference)
            try? api.auth.logout()
            api.transport.session.invalidateAndCancel()
        }
        let controller = NotificationController(transport: api.transport, receiptStore: PreviewPushReceiptStore(),
                                                previewKeys: InMemoryNotificationPreviewKeys())
        controller.connect(api: api, userID: user)
        let delegate = NotificationDelegate()
        delegate.bind(controller)
        let payload = validPayload ? ["user_id": user, "channel_id": UUID().uuidString] : [:]
        try #require(delegate.responds(to: NSSelectorFromString("userNotificationCenter:willPresentNotification:withCompletionHandler:")))
        try await confirmation("通知表示の完了は1回だけ") { completed in
            let options: UNNotificationPresentationOptions = try await awaitCallback { finish in
                DispatchQueue.global().async {
                    #expect(!Thread.isMainThread)
                    do {
                        let notification = try NotificationTestCoder.notification(payload: payload)
                        let receiver: any UNUserNotificationCenterDelegate = delegate
                        receiver.userNotificationCenter?(UNUserNotificationCenter.current(), willPresent: notification) { options in
                            #expect(Thread.isMainThread)
                            completed()
                            finish(.success(options))
                        }
                    } catch { finish(.failure(error)) }
                }
            }
            #expect(options == (validPayload ? [.banner, .sound] : []))
        }
        #expect(controller.destination == nil)
    }

    @Test("通知応答の完了をメインスレッドで一度だけ返し、接続前の宛先も保持する", arguments: [true, false], [true, false])
    func testNotificationResponseCompletion(validPayload: Bool, bindBeforeResponse: Bool) async throws {
        let api = try client()
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel() }
        let controller = NotificationController(transport: api.transport, receiptStore: PreviewPushReceiptStore(),
                                                previewKeys: InMemoryNotificationPreviewKeys())
        let delegate = NotificationDelegate()
        if bindBeforeResponse { delegate.bind(controller) }
        let payload = validPayload ? ["user_id": UUID().uuidString, "channel_id": UUID().uuidString] : [:]
        try #require(delegate.responds(to: NSSelectorFromString("userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:")))
        try await confirmation("通知応答の完了は1回だけ") { completed in
            let _: Void = try await awaitCallback { finish in
                DispatchQueue.global().async {
                    #expect(!Thread.isMainThread)
                    do {
                        let notification = try NotificationTestCoder.notification(payload: payload)
                        let response = try #require(UNNotificationResponse(coder: NotificationTestCoder([
                            "notification": notification, "actionIdentifier": UNNotificationDefaultActionIdentifier
                        ])))
                        let receiver: any UNUserNotificationCenterDelegate = delegate
                        receiver.userNotificationCenter?(UNUserNotificationCenter.current(), didReceive: response) {
                            #expect(Thread.isMainThread)
                            completed()
                            finish(.success(()))
                        }
                    } catch { finish(.failure(error)) }
                }
            }
        }
        if !bindBeforeResponse { delegate.bind(controller) }
        #expect(controller.destination == PushDestination(payload: payload))
    }

    @Test("VoIP トークンの初回通知前は保存済み購読を保ち、明示失効で解除する")
    func preservesUnknownVoIPToken() async throws {
        let api = try client()
        let user = UUID().uuidString
        let account = PushReceipt.digest(api.settings.storageKey + "|" + user)
        let preference = "zoo.push.enabled." + account
        UserDefaults.standard.set(true, forKey: preference)
        let voip = PreviewPushReceiptStore()
        let receipt = PushReceipt(server: api.settings.server, account: account, token: "old-voip",
                                  environment: "sandbox", secret: "test-secret", kind: .voip)
        try voip.save(receipt)
        let controller = NotificationController(transport: api.transport, receiptStore: PreviewPushReceiptStore(),
                                               previewKeys: InMemoryNotificationPreviewKeys(), voipStore: voip)
        defer {
            UserDefaults.standard.removeObject(forKey: preference)
            try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil)
        }
        let revocations = Mutex(0)
        stub { request in
            switch request.url?.path {
            case "/api/push/apns/config": return (200, [:], Data(#"{"sandbox":true,"production":false,"voip":true}"#.utf8))
            case "/api/push/apns/subscriptions":
                return (200, [:], try JSONEncoder().encode(["id": PushReceipt.digest("sandbox:aa")]))
            default:
                #expect(request.url?.path == "/api/push/apns/revoke")
                #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
                revocations.withLock { $0 += 1 }
                return (200, [:], Data(#"{"ok":true}"#.utf8))
            }
        }
        controller.connect(api: api, userID: user)
        await controller.received(token: Data([0xaa]))?.value
        #expect(try voip.load() == receipt)
        #expect(revocations.withLock { $0 } == 0)
        await controller.receivedVoIPToken(nil)?.value
        #expect(try voip.load() == nil)
        #expect(revocations.withLock { $0 } == 1)
    }

    @Test("通知の投稿 ID は省略とnullを許可し、不正な値は拒否する")
    func rejectsMalformedMessageID() {
        let base: [AnyHashable: Any] = ["user_id": UUID().uuidString, "channel_id": UUID().uuidString]
        #expect(PushDestination(payload: base) != nil)
        var channelOnly = base
        channelOnly["message_id"] = NSNull()
        #expect(PushDestination(payload: channelOnly) == PushDestination(payload: base))
        #expect(PushNotice(payload: channelOnly) != nil)
        for invalid in [42, "", "invalid", ["id"]] as [Any] {
            var payload = base
            payload["message_id"] = invalid
            #expect(PushDestination(payload: payload) == nil)
        }
    }

    @Test func testUnauthorizedIsAnExpiredSession() async throws {
        let api = try client()
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        stub { _ in (401, [:], Data()) }
        do { let _: Person = try await api.get("api/users/me"); Issue.record("認証エラーの応答が受け入れられました。") }
        catch ZooError.expiredSession { }
    }

    @Test func testRefreshIsSharedAcrossConcurrentRequests() async throws {
        let api = try client(expired: true)
        let release = AsyncStream<Void>.makeStream()
        let exchanges = Mutex(0)
        defer { release.continuation.finish(); try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        stub { request in
            if request.url?.path == "/.well-known/openid-configuration" { return Self.discoveryResponse }
            #expect(request.url?.path == "/token")
            let count = exchanges.withLock { $0 += 1; return $0 }
            guard count == 1 else { Issue.record("更新トークンの交換が重複しました。"); throw ZooError.invalidResponse }
            for await _ in release.stream { break }
            return Self.tokenResponse
        }
        var completed = 0
        let first = Task { defer { completed += 1 }; return try await api.auth.accessToken() }
        let second = Task { defer { completed += 1 }; return try await api.auth.accessToken() }
        defer { first.cancel(); second.cancel() }
        let deadline = ContinuousClock.now.advanced(by: .seconds(10))
        while (api.auth.refreshWaiterCount < 2 || exchanges.withLock({ $0 }) == 0), completed == 0, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(1))
        }
        try #require(api.auth.refreshWaiterCount == 2 && completed == 0, "更新待機が揃いませんでした。waiters=\(api.auth.refreshWaiterCount)、completed=\(completed)")
        #expect(exchanges.withLock { $0 } == 1)
        release.continuation.finish()
        let values = try await [first.value, second.value]
        #expect(values == ["renewed", "renewed"])
        #expect(exchanges.withLock { $0 } == 1)
        #expect(try TokenVault.load(api.settings.storageKey)?.refresh == "rotated")
    }

    @Test("更新トークンがなくても実際の有効期限まではアクセストークンを使う")
    func accessTokenWithoutRefreshUsesRemainingLifetime() async throws {
        let api = try client()
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        try TokenVault.save(TokenSet(access: "remaining", refresh: nil, expires: .now.addingTimeInterval(20)), account: api.settings.storageKey)
        #expect(try await api.auth.accessToken() == "remaining")
    }

    @Test("更新後の保存失敗は通信を繰り返さず保存だけ再試行する")
    func retriesSavingRotatedTokens() async throws {
        let api = try client(expired: true)
        var saves = 0
        let auth = Authentication(settings: api.settings, transport: api.transport, saveToken: { token, account in
            saves += 1
            if saves == 1 { throw ZooError.auth("保存失敗の検証") }
            try TokenVault.save(token, account: account)
        })
        defer { try? auth.logout(); try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        try await confirmation("更新トークンを交換するのは一度だけ") { exchanged in
            stub { request in
                if request.url?.path == "/.well-known/openid-configuration" { return Self.discoveryResponse }
                exchanged()
                return Self.tokenResponse
            }
            await #expect(throws: ZooError.self) { try await auth.accessToken() }
            let access = try await auth.accessToken()
            let stored = try TokenVault.load(api.settings.storageKey)
            #expect(access == "renewed")
            #expect(saves == 2)
            #expect(stored?.refresh == "rotated")
        }
    }

    @Test("設定エラーではセッションを失効させず、invalid_grant の場合だけ失効を返す",
          arguments: ["invalid_client", "invalid_grant", "unknown_error"])
    func classifiesRefreshErrors(code: String) async throws {
        let api = try client(expired: true)
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        stub { request in
            if request.url?.path == "/.well-known/openid-configuration" { return Self.discoveryResponse }
            return (400, [:], try JSONEncoder().encode(["error": code]))
        }
        do {
            _ = try await api.auth.accessToken()
            Issue.record("失敗した更新要求が受け入れられました。")
        } catch ZooError.expiredSession {
            #expect(code == "invalid_grant")
        } catch ZooError.auth {
            #expect(code != "invalid_grant")
        }
        #expect(try TokenVault.load(api.settings.storageKey)?.refresh == "test-refresh")
    }

    @Test("Discoveryの失敗では更新トークンを保持し、復旧後に更新できる", arguments: [401, 403, 503])
    func discoveryFailurePreservesSession(status: Int) async throws {
        let api = try client(expired: true)
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        stub { request in
            #expect(request.url?.path == "/.well-known/openid-configuration")
            return (status, [:], Data())
        }
        do {
            _ = try await api.auth.accessToken()
            Issue.record("Discoveryの失敗が受け入れられました。")
        } catch ZooError.auth { }
        #expect(try TokenVault.load(api.settings.storageKey)?.refresh == "test-refresh")
        stub { request in
            request.url?.path == "/.well-known/openid-configuration" ? Self.discoveryResponse : Self.tokenResponse
        }
        #expect(try await api.auth.accessToken() == "renewed")
    }

    @Test("設定取得中にログアウトした購読を新しい認証情報で登録しない")
    func subscriptionStopsAfterSessionChange() async throws {
        let api = try client()
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
        let backend = LivePushBackend(api: api, transport: api.transport)
        let receipt = PushReceipt(server: api.settings.server, account: "old", token: "test-token",
                                  environment: "sandbox", secret: "test-secret")
        stub { request in
            #expect(request.url?.path == "/api/push/apns/config")
            try? await api.auth.logout()
            return (200, [:], Data(#"{"sandbox":true,"production":false}"#.utf8))
        }
        await #expect(throws: CancellationError.self) { try await backend.subscribe(receipt) }
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
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
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
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
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
            let first = Task { defer { started.continuation.finish() }; return try await api.auth.accessToken() }
            defer { first.cancel() }
            var startedIterator = started.stream.makeAsyncIterator()
            try #require(await startedIterator.next() != nil)
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
                } catch ZooError.auth { }
                #expect(try TokenVault.load(api.settings.storageKey)?.refresh == "test-refresh")
            }
        }
    }

    @Test func logoutInvalidatesAnInFlightRefresh() async throws {
        let api = try client(expired: true)
        defer { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); stub(nil) }
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
        let request = Task { defer { started.continuation.finish() }; return try await api.auth.accessToken() }
        defer { request.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        try? api.auth.logout()
        release.continuation.yield(())
        await #expect(throws: CancellationError.self) { try await request.value }
        #expect(try !api.auth.hasSession)
        #expect(try TokenVault.load(api.settings.storageKey) == nil)
    }
}

private final class FailedBodyProtocol: URLProtocol, @unchecked Sendable {
    private static let active = Mutex<[URL: FailedBodyProtocol]>([:])
    private let callbackLock = NSRecursiveLock()
    private let stopped = Mutex(false)

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        guard let url = request.url, let status = Int(url.lastPathComponent),
              let response = HTTPURLResponse(url: url, statusCode: status, httpVersion: "HTTP/1.1",
                                             headerFields: ["Retry-After": "12"]) else { return }
        callbackLock.withLock {
            guard !stopped.withLock({ $0 }) else { return }
            Self.register(self, url: url)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data(repeating: 0xA5, count: 16 * 1024))
        }
    }
    override func stopLoading() {
        callbackLock.withLock {
            stopped.withLock { $0 = true }
            if let url = request.url { _ = Self.active.withLock { $0.removeValue(forKey: url) } }
        }
    }
    private static func register(_ instance: FailedBodyProtocol, url: URL) {
        active.withLock { (entries: inout [URL: FailedBodyProtocol]) in
            entries[url] = instance
        }
    }
    static func fail(_ url: URL) {
        guard let instance = active.withLock({ $0[url] }) else { return }
        instance.callbackLock.withLock {
            guard !instance.stopped.withLock({ $0 }) else { return }
            instance.client?.urlProtocol(instance, didFailWithError: URLError(.networkConnectionLost))
        }
    }
}

@MainActor
@Suite("HTTP応答の一括取得", .timeLimit(.minutes(1)))
struct HTTPResponseTests {
    @Test("受信上限を超える成功応答を拒否し、エラー応答は状態と上限内の本文を保つ", arguments: [200, 401, 403, 503])
    func boundsResponse(status: Int) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in (status, ["Retry-After": "12"], Data(repeating: 65, count: 4096)) }
        let transport = HTTPTransport(session: harness.api.transport.session, maximumResponseBytes: 1024)
        let request = URLRequest(url: URL(string: "https://api.test.invalid/bounded")!)
        if status == 200 {
            do { _ = try await transport.send(request); Issue.record("受信上限を超えた応答が成功しました。") }
            catch { #expect((error as? URLError)?.code == .dataLengthExceedsMaximum) }
        } else {
            let (data, response) = try await transport.send(request)
            #expect(data.count == 1024 && response.statusCode == status)
            #expect(response.value(forHTTPHeaderField: "Retry-After") == "12")
            #expect((try #require(response as? IncompleteHTTPResponse).bodyReadError as? URLError)?.code == .dataLengthExceedsMaximum)
            if status == 401 {
                do { try HTTPTransport.check(data, response); Issue.record("401が認証切れになりませんでした。") }
                catch ZooError.expiredSession { }
            }
        }
    }

    @Test("メモリー受信上限より大きな添付を一時ファイルへ保存する")
    func downloadsToFile() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let body = Data(repeating: 0xA5, count: 256 * 1024)
        harness.stub { _ in (200, [:], body) }
        let transport = HTTPTransport(session: harness.api.transport.session, maximumResponseBytes: 1024)
        let (file, response) = try await transport.download(URLRequest(url: URL(string: "https://api.test.invalid/file")!))
        #expect(response.statusCode == 200)
        #expect(try Data(contentsOf: file.url) == body)
    }

    @Test("50MiBの応答を再要求せず、そのまま取得する")
    func largeResponse() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let payload = Data(repeating: 0xA5, count: 50 * 1024 * 1024)
        let requests = Mutex(0)
        harness.stub { _ in
            requests.withLock { $0 += 1 }
            return (200, [:], payload)
        }
        let request = URLRequest(url: URL(string: "https://api.test.invalid/large")!)
        let start = ContinuousClock.now
        let (data, response) = try await harness.api.transport.send(request)
        print("50MiB応答の取得時間: \(start.duration(to: .now))")
        #expect(response.statusCode == 200 && data == payload)
        #expect(requests.withLock { $0 } == 1)
    }

    @Test("本文が途中で失敗してもエラー応答のヘッダーを保ち、成功応答は失敗として返す",
          arguments: [200, 401, 403, 429, 503])
    func preservesFailedResponse(status: Int) async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [FailedBodyProtocol.self]
        let transport = HTTPTransport(session: URLSession(configuration: config))
        defer { transport.session.invalidateAndCancel() }
        let url = URL(string: "https://api.test.invalid/\(UUID().uuidString)/\(status)")!
        let sending = Task { try await transport.send(URLRequest(url: url)) }
        defer { sending.cancel() }
        // 即時失敗では Foundation がヘッダーも破棄するため、受信を確認してから本文を失敗させる。
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        var received = false
        while !received && ContinuousClock.now < deadline {
            received = await transport.session.allTasks.contains { $0.response != nil }
            if !received { try await Task.sleep(for: .milliseconds(10)) }
        }
        try #require(received)
        FailedBodyProtocol.fail(url)
        if status == 200 {
            do {
                _ = try await sending.value
                Issue.record("途中で失敗した成功応答を受け入れました")
            } catch { #expect((error as? URLError)?.code == .networkConnectionLost) }
        } else {
            let (data, response) = try await sending.value
            #expect(data == Data(repeating: 0xA5, count: 16 * 1024) && response.statusCode == status)
            #expect(response.value(forHTTPHeaderField: "Retry-After") == "12")
            let incomplete = try #require(response as? IncompleteHTTPResponse)
            #expect((incomplete.bodyReadError as? URLError)?.code == .networkConnectionLost)
        }
    }

    @Test("送信前と受信中のキャンセルで通信を終了する", arguments: [false, true])
    func cancelsTransfer(beforeStart: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), stopped = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); stopped.continuation.finish() }
        let requests = Mutex(0)
        harness.stub { _ in
            requests.withLock { $0 += 1 }
            started.continuation.yield(())
            defer { stopped.continuation.yield(()) }
            try await Task.sleep(for: .seconds(60))
            return (200, [:], Data())
        }
        let request = URLRequest(url: URL(string: "https://api.test.invalid/cancel")!)
        let sending = Task { try await harness.api.transport.send(request) }
        defer { sending.cancel() }
        if !beforeStart {
            var iterator = started.stream.makeAsyncIterator()
            try #require(await iterator.next() != nil)
        }
        sending.cancel()
        do {
            _ = try await sending.value
            Issue.record("取り消した通信が成功しました")
        } catch { #expect(error is CancellationError || (error as? URLError)?.code == .cancelled) }
        if beforeStart {
            #expect(requests.withLock { $0 } == 0)
        } else {
            var iterator = stopped.stream.makeAsyncIterator()
            #expect(await iterator.next() != nil)
        }
    }

}
