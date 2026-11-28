import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct AuthenticationTests {
    private struct FailingPreviewInvalidation: NotificationPreviewKeyStorage {
        static let message = "通知鍵を無効化できませんでした。端末のロック解除後に再試行してください。"
        func load() -> NotificationPreviewKey? { nil }
        func save(_ key: NotificationPreviewKey?) throws { throw ZooError.auth(Self.message) }
    }

    @Test("通常の認証読出し失敗では有効な通常通知とVoIP登録を解除しない")
    func transientTokenReadPreservesRegistrations() async throws {
        let transport = transport(), receipts = PreviewPushReceiptStore(), voip = PreviewPushReceiptStore()
        defer { transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        let receipt = PushReceipt(server: settings.server, account: "利用者", token: "aabb", environment: "sandbox", secret: "テスト用解除情報")
        var voiceReceipt = receipt
        voiceReceipt.kind = .voip
        try receipts.save(receipt)
        try voip.save(voiceReceipt)
        let keys = InMemoryNotificationPreviewKeys()
        let key = NotificationPreviewKey(userID: UUID().uuidString, secret: receipt.secret)
        keys.save(key)
        let requests = Mutex(0)
        stub { _ in requests.withLock { $0 += 1 }; return (500, [:], Data()) }
        let auth = Authentication(settings: settings, transport: transport, loadToken: { _ in throw CocoaError(.fileReadNoPermission) })
        let session = AppSession(settings: settings, transport: transport, authentication: auth,
                                 notificationStore: receipts, previewKeys: keys, voipStore: voip)
        await session.resume()
        #expect(!session.isStarting)
        #expect(session.error == CocoaError(.fileReadNoPermission).localizedDescription)
        #expect(try receipts.load()?.revocationPending != true)
        #expect(try voip.load()?.revocationPending != true)
        #expect(keys.load() == key)
        #expect(requests.withLock { $0 } == 0)
    }

    @Test("ログアウト済み認証の削除失敗は通知解除の応答前に警告を表示し、新しいログインだけを待たせる")
    func loggedOutAuthenticationErrorDoesNotWaitForNetwork() async throws {
        let transport = transport(), receipts = PreviewPushReceiptStore()
        try receipts.save(PushReceipt(server: settings.server, account: "利用者", token: "aabb", environment: "sandbox", secret: "テスト用解除情報"))
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer {
            release.continuation.finish(); started.continuation.finish()
            transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID)
        }
        stub { request in
            if request.url?.path == "/api/push/apns/revoke" {
                started.continuation.yield(())
                for await _ in release.stream { break }
                return (200, [:], Data(#"{"ok":true}"#.utf8))
            }
            return (400, [:], Data())
        }
        let authMessage = "ログアウト済み認証の削除に失敗しました。"
        let auth = Authentication(settings: settings, transport: transport, loadToken: { _ in
            throw LoggedOutSessionCleanupError(cause: ZooError.auth(authMessage))
        })
        let session = AppSession(settings: settings, transport: transport, authentication: auth,
                                 notificationStore: receipts, previewKeys: FailingPreviewInvalidation(), voipStore: PreviewPushReceiptStore())
        await session.resume()
        #expect(!session.isStarting)
        #expect(session.error?.contains(authMessage) == true)
        #expect(session.error?.contains(FailingPreviewInvalidation.message) == true)
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        let login = try #require(session.startLogin())
        let deadline = ContinuousClock.now + .seconds(10)
        while !session.waitingForCacheCleanup, session.connecting, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(1))
        }
        #expect(session.waitingForCacheCleanup)
        release.continuation.finish()
        await login.value
    }

    @Test("通知解除の応答待ちでも旧アカウントのキャッシュ消去は進み、キャッシュ読出し障害からの復旧後も消去する", arguments: [false, true])
    func cacheCleanupDoesNotWaitForNotificationResponse(initialOpenFails: Bool) async throws {
        let transport = transport(), receipts = PreviewPushReceiptStore()
        let cache = try await SyncCache.open(inMemory: true)
        let connection = SyncCache.connectionKey(settings)
        let lease = try await cache.activate(connection: connection, user: Person(id: UUID().uuidString, user_name: "test", display_name: "テスト"))
        try await cache.store(Data("以前の本文".utf8), resource: "messages", channelID: UUID().uuidString, lease: lease)
        try receipts.save(PushReceipt(server: settings.server, account: "利用者", token: "aabb", environment: "sandbox", secret: "テスト用解除情報"))
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer {
            release.continuation.finish(); started.continuation.finish()
            transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID)
        }
        stub { request in
            #expect(request.url?.path == "/api/push/apns/revoke")
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, [:], Data(#"{"ok":true}"#.utf8))
        }
        let opens = Mutex(0)
        let session = AppSession(settings: settings, transport: transport,
                                 notificationStore: receipts, previewKeys: InMemoryNotificationPreviewKeys(),
                                 syncCache: initialOpenFails ? nil : cache, openSyncCache: {
                                     let attempt = opens.withLock { $0 += 1; return $0 }
                                     if initialOpenFails && attempt == 1 { throw CocoaError(.fileReadNoPermission) }
                                     return cache
                                 }, voipStore: PreviewPushReceiptStore())
        if initialOpenFails { await session.resume() }
        let resuming = Task { await session.resume() }
        defer { resuming.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        let deadline = ContinuousClock.now + .seconds(10)
        while try await cache.profile(connection: connection) != nil, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(1))
        }
        #expect(try await cache.profile(connection: connection) == nil)
        await #expect(throws: CancellationError.self) { try await cache.load("messages", lease: lease) }
        release.continuation.finish()
        await resuming.value
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

    private func stub(_ handler: @escaping StubProtocol.Handler) { StubProtocol.install(handler, id: stubID) }

    @Test("キャンセル直後の再試行を古い接続タスクが妨げない")
    func appSessionCanRetryImmediatelyAfterCancellation() async throws {
        let transport = transport()
        let session = AppSession(settings: settings, transport: transport, notificationStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys(), voipStore: PreviewPushReceiptStore())
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
        let retry = try #require(session.startLogin())
        let completion = Task { await retry.value; requested.continuation.finish() }
        defer { completion.cancel() }
        var iterator = requested.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        session.cancelLogin()
        #expect(!session.connecting)
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
        let controller = NotificationController(transport: transport, receiptStore: receipts, previewKeys: keys, voipStore: PreviewPushReceiptStore())
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
        let session = AppSession(settings: settings, transport: transport, notificationStore: receipts, previewKeys: keys, voipStore: PreviewPushReceiptStore())
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
        let login = try #require(session.startLogin())
        let completion = Task { await login.value; requests.continuation.finish() }
        defer { login.cancel(); completion.cancel() }
        #expect(keys.load() == nil)
        var iterator = requests.stream.makeAsyncIterator()
        let first = try #require(await iterator.next()), second = try #require(await iterator.next())
        #expect(Set([first, second]) == ["/api/push/apns/revoke", "/.well-known/openid-configuration"])
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
        let session = AppSession(settings: settings, transport: transport, notificationStore: receipts, previewKeys: InMemoryNotificationPreviewKeys(), voipStore: PreviewPushReceiptStore())
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
                                 previewKeys: InMemoryNotificationPreviewKeys(), voipStore: PreviewPushReceiptStore())
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
        let login = try #require(session.startLogin())
        var finished = false
        let completion = Task { await login.value; finished = true }
        defer { completion.cancel() }
        let waitingDeadline = ContinuousClock.now.advanced(by: .seconds(10))
        while !session.waitingForCacheCleanup, !finished, ContinuousClock.now < waitingDeadline {
            try await Task.sleep(for: .milliseconds(1))
        }
        try #require(session.waitingForCacheCleanup && !finished, "キャッシュ消去待ちへ到達しませんでした。waiting=\(session.waitingForCacheCleanup)、finished=\(finished)")
        #expect(discoveryCount.withLock { $0 } == 0)
        release.continuation.finish()
        await resuming.value
        let deadline = ContinuousClock.now + .seconds(10)
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
        }, voipStore: PreviewPushReceiptStore())
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
        let deadline = ContinuousClock.now + .seconds(10)
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
                                 previewKeys: InMemoryNotificationPreviewKeys(), syncCache: cache, voipStore: PreviewPushReceiptStore())
        defer { session.logout(); transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        stub { _ in requests.withLock { $0 += 1 }; return (400, [:], Data()) }
        await session.resume()
        #expect(session.error == CocoaError(.fileWriteOutOfSpace).localizedDescription)
        session.startLogin()
        var deadline = ContinuousClock.now + .seconds(10)
        while session.connecting && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        try #require(!session.connecting)
        #expect(session.error == CocoaError(.fileWriteOutOfSpace).localizedDescription)
        #expect(saves.withLock { $0 } == 1)
        #expect(requests.withLock { $0 } == 0)
        session.startLogin()
        deadline = ContinuousClock.now + .seconds(10)
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
        let session = AppSession(settings: settings, transport: transport, notificationStore: receipts, previewKeys: InMemoryNotificationPreviewKeys(), voipStore: PreviewPushReceiptStore())
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
        let session = AppSession(settings: settings, transport: transport, notificationStore: receipts, previewKeys: InMemoryNotificationPreviewKeys(), voipStore: PreviewPushReceiptStore())
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
