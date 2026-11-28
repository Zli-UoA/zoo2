import Foundation
import Synchronization
import Testing
@testable import Zoo

private let syncUser = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "sakura", display_name: "さくら")
private let syncChannel = "11111111-1111-4111-8111-111111111111"

private final class FaultingCacheRevocations: CacheRevocationStorage {
    let base: KeychainCacheRevocations
    let failures = Mutex((read: false, write: false))
    init(url: URL) { base = KeychainCacheRevocations(namespace: url.appendingPathExtension("permissions").path) }
    func contains(_ scope: String) throws -> Bool {
        if failures.withLock({ $0.read }) { throw CocoaError(.fileReadNoPermission) }
        return try base.contains(scope)
    }
    func revoke(_ scope: String) throws {
        if failures.withLock({ $0.write }) { throw CocoaError(.fileWriteNoPermission) }
        try base.revoke(scope)
    }
    func allow(_ scope: String) throws {
        if failures.withLock({ $0.write }) { throw CocoaError(.fileWriteNoPermission) }
        try base.allow(scope)
    }
}

private func syncResponse(offset: String = "100_0", status: Int = 200) throws -> HTTPURLResponse {
    try #require(HTTPURLResponse(url: URL(string: "https://api.test.invalid/api/shapes/messages")!, statusCode: status,
                                httpVersion: nil, headerFields: ["electric-handle": "h", "electric-offset": offset]))
}

private func syncRow(_ number: Int, parent: String? = nil) -> [String: JSONValue] {
    ["id": .string(String(format: "22222222-2222-4222-8222-%012d", number)), "channel_id": .string(syncChannel),
     "user_id": .string(syncUser.id), "content": .string("投稿\(number)"), "created_at": .string("2026-11-19 10:00:00"),
     "parent_id": parent.map(JSONValue.string) ?? .null, "display_data": .object(["reactions": .array([]), "reply_count": .number(2)])]
}

private func subsetData(_ numbers: [Int], parent: String? = nil) throws -> Data {
    let rows: [JSONValue] = numbers.map { number in
        .object(["key": .string("k\(number)"), "headers": .object(["operation": .string("insert")]),
                 "value": .object(syncRow(number, parent: parent))])
    }
    return try JSONEncoder().encode(JSONValue.object([
        "data": .array(rows), "metadata": .object([
            "xmin": .string("90"), "xmax": .string("100"), "xip_list": .array([.string("95")]),
            "database_lsn": .string("1000"), "snapshot_mark": .number(1)
        ])
    ]))
}

@Suite("SwiftDataの同期キャッシュ", .timeLimit(.minutes(1)))
struct SyncCacheTests {
    @Test("コンテナの移設後も、論理名で記録した失効を維持する")
    func revocationSurvivesContainerMove() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: folder) }
        let original = folder.appendingPathComponent("original")
        let moved = folder.appendingPathComponent("moved")
        try FileManager.default.createDirectory(at: original, withIntermediateDirectories: true)
        let url = original.appendingPathComponent("test.store")
        let revocations = KeychainCacheRevocations(namespace: "move-test-" + UUID().uuidString)
        let fails = Mutex(false)
        let cache = try await SyncCache.open(url: url, beforeDelete: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        }, revocations: revocations)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data("失効前".utf8), resource: "messages", channelID: syncChannel, lease: lease)
        let directory = try #require(FileManager.default.contentsOfDirectory(
            at: url.appendingPathExtension("permissions"), includingPropertiesForKeys: nil).first)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path) }
        try FileManager.default.setAttributes([.posixPermissions: 0o500], ofItemAtPath: directory.path)
        fails.withLock { $0 = true }
        await #expect(throws: (any Error).self) { try await cache.revokeChannel(syncChannel, lease: lease) }
        try FileManager.default.copyItem(at: original, to: moved)
        let restarted = try await SyncCache.open(url: moved.appendingPathComponent("test.store"), revocations: revocations)
        let next = try await restarted.activate(connection: "connection", user: syncUser)
        #expect(try await restarted.load("messages", lease: next) == nil)
        // 移設先の読取り専用ディレクトリも、テスト終了時に削除できるよう戻す。
        let movedPermission = moved.appendingPathComponent("test.store.permissions").appendingPathComponent(directory.lastPathComponent)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: movedPermission.path)
    }

    @Test("Keychainの失効確認ができない間は、保存済み許可だけで復元しない")
    func unreadableRevocationsBlockRestoration() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: folder) }
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent("test.store")
        let revocations = FaultingCacheRevocations(url: url)
        let cache = try await SyncCache.open(url: url, revocations: revocations)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data("保存済み".utf8), resource: "messages", channelID: syncChannel, lease: lease)
        let attachmentScope = "attachment-files:" + syncChannel
        try await cache.store(Data("添付の保存先".utf8), resource: attachmentScope, channelID: syncChannel, lease: lease)
        let notifications = "snapshot:api/shapes/notifications"
        try await cache.store(Data("以前の通知".utf8), resource: notifications, lease: lease)
        revocations.failures.withLock { $0.read = true }
        #expect(try await cache.load("messages", lease: lease) == nil)
        for resource in ["messages", "read-state"] {
            await #expect(throws: CocoaError.self) {
                try await cache.store(Data("新しい値".utf8), resource: resource, channelID: syncChannel, lease: lease)
            }
        }
        await #expect(throws: CocoaError.self) {
            try await cache.store(Data("新しい通知".utf8), resource: notifications, lease: lease)
        }
        await #expect(throws: CocoaError.self) { try await cache.profile(connection: "connection") }
        await #expect(throws: CocoaError.self) { try await cache.activate(connection: "connection", user: syncUser) }
        revocations.failures.withLock { $0.read = false }
        #expect(try await cache.load("messages", lease: lease) == Data("保存済み".utf8))
        #expect(try await cache.load(attachmentScope, lease: lease) == Data("添付の保存先".utf8))
        #expect(try await cache.load("read-state", lease: lease) == nil)
        #expect(try await cache.load(notifications, lease: lease) == Data("以前の通知".utf8))
        let reopened = try await SyncCache.open(url: url)
        let restored = try await reopened.activate(connection: "connection", user: syncUser)
        #expect(try await reopened.load("messages", lease: restored) == Data("保存済み".utf8))
        #expect(try await reopened.load(attachmentScope, lease: restored) == Data("添付の保存先".utf8))
    }

    @Test("Keychainの削除だけが失敗しても、失効記録のないアカウントは起動できる")
    func activationWithoutRevocationDoesNotDeleteKeychainItems() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: folder) }
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent("test.store")
        let revocations = FaultingCacheRevocations(url: url)
        let cache = try await SyncCache.open(url: url, revocations: revocations)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data("保存済み".utf8), resource: "messages", channelID: syncChannel, lease: lease)
        revocations.failures.withLock { $0.write = true }
        let reopened = try await SyncCache.open(url: url, revocations: revocations)
        let restored = try await reopened.activate(connection: "connection", user: syncUser)
        #expect(try await reopened.load("messages", lease: restored) == Data("保存済み".utf8))
        // 実際に失効済みなら、解除できるまでは起動を拒否する。
        try revocations.base.revoke(lease.account + "|account")
        await #expect(throws: CocoaError.self) { try await reopened.activate(connection: "connection", user: syncUser) }
        revocations.failures.withLock { $0.write = false }
        let recovered = try await reopened.activate(connection: "connection", user: syncUser)
        #expect(try await reopened.load("messages", lease: recovered) == nil)
    }

    @Test("全保存先の同時失敗は再起動後を保証できず、復旧後に失効を再試行する")
    func allRevocationStoresFail() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: folder) }
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent("test.store")
        let revocations = FaultingCacheRevocations(url: url)
        let fails = Mutex(false)
        let cache = try await SyncCache.open(url: url, beforeDelete: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        }, revocations: revocations)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data("失効前".utf8), resource: "messages", channelID: syncChannel, lease: lease)
        let directory = try #require(FileManager.default.contentsOfDirectory(
            at: url.appendingPathExtension("permissions"), includingPropertiesForKeys: nil).first)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path) }
        try FileManager.default.setAttributes([.posixPermissions: 0o500], ofItemAtPath: directory.path)
        fails.withLock { $0 = true }
        revocations.failures.withLock { $0.write = true }
        await #expect(throws: (any Error).self) { try await cache.revokeChannel(syncChannel, lease: lease) }
        #expect(try await cache.load("messages", lease: lease) == nil)
        let restarted = try await SyncCache.open(url: url)
        let next = try await restarted.activate(connection: "connection", user: syncUser)
        #expect(try await restarted.load("messages", lease: next) == Data("失効前".utf8))
        revocations.failures.withLock { $0.write = false }
        fails.withLock { $0 = false }
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
        try await cache.revokeChannel(syncChannel, lease: lease)
        let recovered = try await SyncCache.open(url: url)
        let recoveredLease = try await recovered.activate(connection: "connection", user: syncUser)
        #expect(try await recovered.load("messages", lease: recoveredLease) == nil)
    }

    @Test("ログアウト時に許可とDBを消せなくても、再起動後のプロフィールと旧会話を復元しない")
    func accountRevocationSurvivesDeletionFailures() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: folder) }
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent("test.store")
        let fails = Mutex(false)
        let cache = try await SyncCache.open(url: url, beforeDelete: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data("以前の会話".utf8), resource: "messages", channelID: syncChannel, lease: lease)
        let directory = try #require(FileManager.default.contentsOfDirectory(
            at: url.appendingPathExtension("permissions"), includingPropertiesForKeys: nil).first)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path) }
        try FileManager.default.setAttributes([.posixPermissions: 0o500], ofItemAtPath: directory.path)
        fails.withLock { $0 = true }
        await #expect(throws: (any Error).self) { try await cache.revoke(lease) }
        let restarted = try await SyncCache.open(url: url, beforeDelete: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        #expect(try await restarted.profile(connection: "connection") == nil)
        await #expect(throws: (any Error).self) { try await restarted.activate(connection: "connection", user: syncUser) }
        fails.withLock { $0 = false }
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
        let next = try await restarted.activate(connection: "connection", user: syncUser)
        #expect(try await restarted.load("messages", lease: next) == nil)
        try await restarted.store(Data("新しい会話".utf8), resource: "messages", channelID: syncChannel, lease: next)
        let offline = try await SyncCache.open(url: url)
        let restored = try await offline.activate(connection: "connection", user: syncUser)
        #expect(try await offline.load("messages", lease: restored) == Data("新しい会話".utf8))
    }

    @Test("取消済みの古い復元は現在のアカウントとキャッシュを変更しない")
    func cancelledActivationPreservesCurrentAccount() async throws {
        let cache = try await SyncCache.open(inMemory: true)
        let current = Person(id: "current", user_name: "current", display_name: "現在の利用者")
        let lease = try await cache.activate(connection: "connection", user: current)
        let data = Data("現在の保存済み会話".utf8)
        try await cache.store(data, resource: "messages", lease: lease)
        let stale = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await cache.activate(connection: "connection", user: syncUser)
        }
        await #expect(throws: CancellationError.self) { _ = try await stale.value }
        #expect(try await cache.profile(connection: "connection") == current)
        #expect(try await cache.load("messages", lease: lease) == data)
    }

    @Test("旧形式の削除では読出し後に保存された行を消さない")
    func removesOnlyTheReadRevision() async throws {
        let cache = try await SyncCache.open(inMemory: true)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        let key = "snapshot:api/shapes/notifications"
        let old = Data("旧形式".utf8), fresh = Data("新形式".utf8)
        try await cache.store(old, resource: key, lease: lease)
        let read = try #require(await cache.loadVersioned(key, lease: lease))
        try await cache.store(fresh, resource: key, lease: lease)
        try await cache.remove(key, matching: read.revision, lease: lease)
        #expect(try await cache.load(key, lease: lease) == fresh)
        let current = try #require(await cache.loadVersioned(key, lease: lease))
        try await cache.remove(key, matching: current.revision, lease: lease)
        #expect(try await cache.load(key, lease: lease) == nil)
    }

    @Test("データと同期位置を保存し、コンテナを開き直して復元する")
    func reopensDiskStore() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("test.store")
        let cache = try await SyncCache.open(url: url)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        var state = ShapeState()
        state.handle = "h"; state.offset = "123_4"; state.isLive = true
        state.rows = ["k": syncRow(1)]
        try await cache.store(JSONEncoder().encode(state), resource: "messages", channelID: syncChannel, lease: lease)
        let reopened = try await SyncCache.open(url: url)
        #expect(try await reopened.profile(connection: "connection") == syncUser)
        let newLease = try await reopened.activate(connection: "connection", user: syncUser)
        let bytes = try #require(await reopened.load("messages", lease: newLease))
        let restored = try JSONDecoder().decode(ShapeState.self, from: bytes)
        #expect(restored.offset == "123_4")
        #expect(restored.rows["k"]?["content"] == .string("投稿1"))
    }

    @Test("アカウント切替後に古い処理から保存できず、他人のデータを読めない")
    func separatesAccounts() async throws {
        let cache = try await SyncCache.open(inMemory: true)
        let old = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data("秘密のメッセージ".utf8), resource: "messages", lease: old)
        let other = try await cache.activate(connection: "connection", user: Person(id: "other", user_name: "other", display_name: "別のユーザー"))
        #expect(try await cache.load("messages", lease: other) == nil)
        await #expect(throws: CancellationError.self) { try await cache.store(Data(), resource: "messages", lease: old) }
        await #expect(throws: CancellationError.self) { try await cache.updateProfile(syncUser, lease: old) }
        #expect(try await cache.profile(connection: "connection")?.id == "other")
        try await cache.revoke(other)
        #expect(try await cache.profile(connection: "connection") == nil)
        await #expect(throws: CancellationError.self) { try await cache.load("messages", lease: other) }
    }

    @Test("一覧削除の保存が失敗しても再起動後に隠し、新しい一覧の保存後はオフラインで復元する")
    func revocationSurvivesFailedSaveAndRestart() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("test.store")
        let fails = Mutex(false)
        let cache = try await SyncCache.open(url: url, beforeSave: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        let resources = ["shape:api/shapes/notifications", "snapshot:api/shapes/notifications",
                         "shape:api/shapes/channels", "snapshot:api/shapes/channels", "workspace:channels",
                         "shape:api/shapes/user_pinned_channels", "snapshot:api/shapes/user_pinned_channels",
                         "rest:api/channel-read-states", "rest:api/channel-pins"]
        for resource in resources { try await cache.store(Data("失効前の通知".utf8), resource: resource, lease: lease) }
        let readStateKey = "channel-rest:api/channels/" + syncChannel + "/read-state"
        try await cache.store(Data("失効前の既読位置".utf8), resource: readStateKey, channelID: syncChannel, lease: lease)
        try await cache.store(Data("別の情報".utf8), resource: "other", lease: lease)
        fails.withLock { $0 = true }
        await #expect(throws: CocoaError.self) { try await cache.revokeChannel(syncChannel, lease: lease) }
        for resource in resources { #expect(try await cache.load(resource, lease: lease) == nil) }

        let reopened = try await SyncCache.open(url: url)
        let next = try await reopened.activate(connection: "connection", user: syncUser)
        for resource in resources { #expect(try await reopened.load(resource, lease: next) == nil) }
        #expect(try await reopened.load(readStateKey, lease: next) == nil)
        #expect(try await reopened.load("other", lease: next) == Data("別の情報".utf8))
        for resource in resources { try await reopened.store(Data("再取得後の通知".utf8), resource: resource, lease: next) }

        let offline = try await SyncCache.open(url: url)
        let restored = try await offline.activate(connection: "connection", user: syncUser)
        for resource in resources { #expect(try await offline.load(resource, lease: restored) == Data("再取得後の通知".utf8)) }
    }

    @Test("読出し許可の作成失敗後も再保存を試せば、通知を再起動後に復元できる")
    func retriesPermissionFileCreation() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("test.store")
        let marker = url.appendingPathExtension("permissions")
        let fails = Mutex(false)
        let cache = try await SyncCache.open(url: url, beforeSave: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        let resources = ["shape:api/shapes/notifications", "snapshot:api/shapes/notifications"]
        for resource in resources { try await cache.store(Data("失効前".utf8), resource: resource, lease: lease) }
        fails.withLock { $0 = true }
        await #expect(throws: CocoaError.self) { try await cache.revokeChannel(syncChannel, lease: lease) }
        fails.withLock { $0 = false }

        // 保存先の親をファイルに置き換え、DB 成功後の読出し許可作成だけを失敗させる。
        try FileManager.default.removeItem(at: marker)
        try Data().write(to: marker)
        for resource in resources {
            await #expect(throws: (any Error).self) {
                try await cache.store(Data("再取得後".utf8), resource: resource, lease: lease)
            }
            #expect(try await cache.load(resource, lease: lease) == nil)
        }
        try FileManager.default.removeItem(at: marker)
        try FileManager.default.createDirectory(at: marker, withIntermediateDirectories: false)
        for resource in resources { try await cache.store(Data("再取得後".utf8), resource: resource, lease: lease) }

        let offline = try await SyncCache.open(url: url)
        let restored = try await offline.activate(connection: "connection", user: syncUser)
        for resource in resources { #expect(try await offline.load(resource, lease: restored) == Data("再取得後".utf8)) }
    }

    @Test("許可ファイルとDBの削除失敗の組合せに応じて、再起動後の復元と再試行を確認する",
          arguments: [false, true], [false, true])
    func revocationAcrossDeletionFailures(failsDeletion: Bool, failsPermissionRemoval: Bool) async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("test.store")
        let fails = Mutex(false)
        let cache = try await SyncCache.open(url: url, beforeDelete: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        let notifications = ["shape:api/shapes/notifications", "snapshot:api/shapes/notifications"]
        for key in ["messages", "reactions"] {
            try await cache.store(Data("失効前".utf8), resource: key, channelID: syncChannel, lease: lease)
        }
        for key in notifications { try await cache.store(Data("失効前".utf8), resource: key, lease: lease) }
        try await cache.store(Data("別の会話".utf8), resource: "other", channelID: "other", lease: lease)
        let directory = try #require(FileManager.default.contentsOfDirectory(
            at: url.appendingPathExtension("permissions"), includingPropertiesForKeys: nil).first)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path) }
        if failsPermissionRemoval {
            try FileManager.default.setAttributes([.posixPermissions: 0o500], ofItemAtPath: directory.path)
        }
        fails.withLock { $0 = failsDeletion }
        if failsDeletion || failsPermissionRemoval {
            await #expect(throws: (any Error).self) { try await cache.revokeChannel(syncChannel, lease: lease) }
        } else { try await cache.revokeChannel(syncChannel, lease: lease) }
        for key in ["messages", "reactions"] + notifications { #expect(try await cache.load(key, lease: lease) == nil) }
        #expect(try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil).count
                == (failsPermissionRemoval ? 4 : 1))
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)

        let reopened = try await SyncCache.open(url: url)
        let next = try await reopened.activate(connection: "connection", user: syncUser)
        for key in ["messages", "reactions"] + notifications { #expect(try await reopened.load(key, lease: next) == nil) }
        #expect(try await reopened.load("other", lease: next) == Data("別の会話".utf8))
        try await reopened.store(Data("再取得後".utf8), resource: "messages", channelID: syncChannel, lease: next)
        #expect(try await reopened.load("messages", lease: next) == Data("再取得後".utf8))
        #expect(try await reopened.load("reactions", lease: next) == nil)
        let offline = try await SyncCache.open(url: url)
        let restored = try await offline.activate(connection: "connection", user: syncUser)
        #expect(try await offline.load("messages", lease: restored) == Data("再取得後".utf8))
        #expect(try await offline.load("reactions", lease: restored) == nil)
    }

    @Test("ログアウト・アカウント切替・接続削除で該当する許可ファイルだけを消す",
          arguments: ["logout", "switch", "forget"])
    func removesAccountPermissions(action: String) async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("test.store")
        let cache = try await SyncCache.open(url: url)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data(), resource: "messages", channelID: syncChannel, lease: lease)
        try await cache.store(Data(), resource: "remaining", channelID: "remaining", lease: lease)
        try await cache.revokeChannel(syncChannel, lease: lease)
        let permissions = url.appendingPathExtension("permissions")
        let directory = try #require(FileManager.default.contentsOfDirectory(at: permissions, includingPropertiesForKeys: nil).first)
        let other = try await cache.activate(connection: "other-connection", user: syncUser)
        try await cache.store(Data("別の接続".utf8), resource: "messages", channelID: syncChannel, lease: other)
        switch action {
        case "logout": try await cache.revoke(lease)
        case "switch": _ = try await cache.activate(connection: "connection", user: Person(id: "other", user_name: "other", display_name: "別の利用者"))
        default: try await cache.forget(connection: "connection")
        }
        #expect(!FileManager.default.fileExists(atPath: directory.path))
        #expect(try FileManager.default.contentsOfDirectory(at: permissions, includingPropertiesForKeys: nil).count == 1)
        #expect(try await cache.load("messages", lease: other) == Data("別の接続".utf8))
        let restored = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data("再ログイン後".utf8), resource: "messages", channelID: syncChannel, lease: restored)
        #expect(try await cache.load("messages", lease: restored) == Data("再ログイン後".utf8))
    }

    @Test("権限を失った会話と通知だけを削除する")
    func revokesChannel() async throws {
        let cache = try await SyncCache.open(inMemory: true)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data(), resource: "messages", channelID: syncChannel, lease: lease)
        try await cache.store(Data(), resource: "other", channelID: "other", lease: lease)
        try await cache.store(Data(), resource: "shape:api/shapes/notifications", lease: lease)
        try await cache.revokeChannel(syncChannel, lease: lease)
        #expect(try await cache.load("messages", lease: lease) == nil)
        #expect(try await cache.load("shape:api/shapes/notifications", lease: lease) == nil)
        #expect(try await cache.load("other", lease: lease) != nil)
    }
}

@Suite("Electricの履歴とSSE", .timeLimit(.minutes(1)))
struct ElectricProtocolTests {
    @Test("UTF-8と複数data行をLF・CRLFで読み、コメントを無視する", arguments: ["\n", "\r\n", "\r"])
    func parsesSSE(newline: String) throws {
        let input = [": keep-alive", "", "data: {", "data: \"内容\":\"さくら🌸\"}", "", "data: 未完了"]
            .joined(separator: newline)
        var parser = ElectricSSEParser()
        var values: [Data] = []
        for byte in input.utf8 { if let value = try parser.feed(byte) { values.append(value) } }
        #expect(values.count == 1)
        #expect(try JSONDecoder().decode([String: String].self, from: #require(values.first)) == ["内容": "さくら🌸"])
    }

    @Test("subsetに含まれる古い変更を除外し、実行中だった変更は反映する")
    func snapshotVisibility() throws {
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "80_0"
        _ = try state.addSnapshot(subsetData([1]), response: syncResponse())
        let old = Data(#"[{"key":"k1","headers":{"operation":"update","txids":[94]},"value":{"content":"古い本文"}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        try state.apply(old, response: syncResponse())
        #expect(state.shape.rows["k1"]?["content"] == .string("投稿1"))
        let concurrent = Data(#"[{"key":"k1","headers":{"operation":"update","txids":[95]},"value":{"content":"同時更新"}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        try state.apply(concurrent, response: syncResponse(offset: "101_0"))
        #expect(state.shape.rows["k1"]?["content"] == .string("同時更新"))
        let data = try JSONEncoder().encode(state)
        #expect(try JSONDecoder().decode(MessageSyncState.self, from: data).boundaries.first?.keys == ["k1"])
    }

    @Test("取得範囲外の古い投稿を保存せず、読んでいる範囲の新着を反映する")
    func limitsLiveRows() throws {
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "80_0"
        state.windows["root"] = MessageWindow(parentID: nil, limit: 50, hasMore: true,
                                               oldestDate: "2026-11-19 10:00:00", oldestID: syncRow(50)["id"]?.text)
        let events: [JSONValue] = [1, 100].map { number in
            .object(["key": .string("k\(number)"), "headers": .object(["operation": .string("update")]), "value": .object(syncRow(number))])
        }
        try state.apply(JSONEncoder().encode(events), response: syncResponse())
        #expect(state.shape.rows.keys.sorted() == ["k100"])
    }

    @Test("409で履歴とスナップショット境界を破棄する")
    func resetsWindow() throws {
        var state = MessageSyncState()
        state.shape.handle = "h"
        _ = try state.addSnapshot(subsetData([1]), response: syncResponse())
        try state.apply(Data(), response: syncResponse(status: 409))
        #expect(state.shape.rows.isEmpty)
        #expect(state.boundaries.isEmpty)
        #expect(state.shape.offset == "now")
    }

    @Test("所属変更をタグのOR・AND条件に適用し、ログを巻き戻さない")
    func appliesMovesWithPersistedTags() throws {
        var state = ShapeState()
        let rows = Data(#"[{"key":"private","value":{"id":"private"},"headers":{"operation":"insert","tags":["member/","/1"],"active_conditions":[true,false]}},{"key":"public","value":{"id":"public"},"headers":{"operation":"insert","tags":["member/","/1"],"active_conditions":[true,true]}},{"key":"and","value":{"id":"and"},"headers":{"operation":"insert","tags":["member/other"],"active_conditions":[true,true]}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(rows, response: syncResponse(), as: JSONValue.self)
        state = try JSONDecoder().decode(ShapeState.self, from: JSONEncoder().encode(state))
        let moveOut = Data(#"[{"headers":{"event":"move-out","patterns":[{"pos":0,"value":"member"}]}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(moveOut, response: syncResponse(offset: "101_0"), as: JSONValue.self)
        #expect(state.rows.keys.sorted() == ["public"])
        #expect(state.visibility["public"]?.conditions == [false, true])
        #expect(state.offset == "101_0")
        #expect(state.handle == "h")
        let moveIn = Data(#"[{"headers":{"event":"move-in","patterns":[{"pos":0,"value":"member"}]}},{"headers":{"event":"move-out","patterns":[{"pos":1,"value":"1"}]}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(moveIn, response: syncResponse(offset: "102_0"), as: JSONValue.self)
        #expect(state.rows.keys.sorted() == ["public"])
        #expect(state.visibility["public"]?.conditions == [true, false])
        _ = try state.apply(moveOut, response: syncResponse(offset: "103_0"), as: JSONValue.self)
        #expect(state.rows.isEmpty)
        #expect(state.visibility.isEmpty)
        #expect(state.offset == "103_0")
    }

    @Test("条件配列のないタグも追加と削除を反映する", arguments: [false, true])
    func appliesSimpleTags(update: Bool) throws {
        var state = ShapeState()
        let rows = Data(#"[{"key":"k","value":{"id":"k"},"headers":{"operation":"insert","tags":["a/","b/"]}},{"key":"k","value":{"id":"k"},"headers":{"operation":"update","tags":["c/"],"removed_tags":["a/"]}},{"headers":{"event":"move-out","patterns":[{"pos":0,"value":"b"}]}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(rows, response: syncResponse(), as: JSONValue.self)
        #expect(state.visibility["k"]?.tags == ["c/"])
        #expect(state.rows["k"] != nil)
        let move = Data((update
            ? #"[{"key":"k","value":{"id":"k"},"headers":{"operation":"update","removed_tags":["c/"]}},{"headers":{"control":"up-to-date"}}]"#
            : #"[{"headers":{"event":"move-out","patterns":[{"pos":0,"value":"c"}]}},{"headers":{"control":"up-to-date"}}]"#).utf8)
        _ = try state.apply(move, response: syncResponse(offset: "101_0"), as: JSONValue.self)
        #expect(state.rows.isEmpty)
    }
}

@MainActor
@Suite("永続同期の通信経路", .timeLimit(.minutes(1)))
struct PersistentSyncIntegrationTests {
    @Test("ピン操作直後の再起動でも確定済み一覧を復元し、遅れたGETで保存を巻き戻さない",
          arguments: [false, true])
    func pinWritesPersistAcrossRestart(pinned: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let channel = Channel(id: syncChannel, channel_name: "保存済み", is_private: false, is_dm: false)
        let oldPins = ChannelPins(pins: pinned ? [] : [ChannelPin(channel_id: syncChannel, position: 0)], version: 1)
        let newPins = ChannelPins(pins: pinned ? [ChannelPin(channel_id: syncChannel, position: 0)] : [], version: 2)
        try await sync.store([channel], key: "workspace:channels")
        try await sync.store([syncUser], key: "workspace:people")
        try await sync.store([ChannelReadState(channel_id: syncChannel, has_unread: false, last_read_at: nil)], key: "rest:api/channel-read-states")
        try await sync.store(oldPins, key: "rest:api/channel-pins")
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let requests = Mutex(0)
        harness.stub { request in
            requests.withLock { $0 += 1 }
            if request.httpMethod == "GET" {
                #expect(request.url?.path == "/api/channel-pins")
                started.continuation.yield(())
                for await _ in release.stream { break }
                return (200, [:], try JSONEncoder().encode(oldPins))
            }
            #expect(request.httpMethod == (pinned ? "POST" : "DELETE"))
            return (200, [:], try JSONEncoder().encode(["data": newPins]))
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        let pending = Task { try await client.channelPins(allowOfflineFallback: false) }
        defer { pending.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        _ = try await client.setChannelPinned(id: syncChannel, pinned: pinned, expectedVersion: 1)
        #expect(try await sync.load("rest:api/channel-pins", as: ChannelPins.self)?.version == 2)
        release.continuation.finish()
        _ = try await pending.value
        let restored = try #require(try await client.cachedWorkspace())
        #expect(restored.pins?.version == 2)
        #expect(restored.pins?.pins == newPins.pins)
        #expect(requests.withLock { $0 } == 2)
    }

    @Test("参加状態が未保存・破損なら空の参加一覧として復元しない", arguments: [false, true])
    func incompleteWorkspaceCache(corrupt: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        try await sync.store([Channel(id: syncChannel, channel_name: "参加済み", is_private: false, is_dm: false)], key: "workspace:channels")
        try await sync.store([syncUser], key: "workspace:people")
        if corrupt { try await sync.store("旧形式", key: "rest:api/channel-read-states") }
        let requests = Mutex(0)
        harness.stub { _ in
            requests.withLock { $0 += 1 }
            throw URLError(.notConnectedToInternet)
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        #expect(try await client.cachedWorkspace() == nil)
        #expect(requests.withLock { $0 } == 0)
        let workspace = WorkspaceStore(client: client, user: syncUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        await workspace.refresh()
        #expect(!workspace.hasLoaded)
        #expect(workspace.error != nil)
    }

    @Test("参加状態とピンを通信前に一緒に復元し、未取得と空のピンを区別する",
          arguments: [false, true], [nil, false, true] as [Bool?])
    func completeWorkspaceCache(joined: Bool, pinned: Bool?) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let channel = Channel(id: syncChannel, channel_name: "保存済み", is_private: false, is_dm: false)
        let states = joined ? [ChannelReadState(channel_id: syncChannel, has_unread: true, last_read_at: nil)] : []
        try await sync.store([channel], key: "workspace:channels")
        try await sync.store([syncUser], key: "workspace:people")
        try await sync.store(states, key: "rest:api/channel-read-states")
        if let pinned {
            try await sync.store(ChannelPins(pins: pinned ? [ChannelPin(channel_id: syncChannel, position: 0)] : [], version: 3),
                                 key: "rest:api/channel-pins")
        }
        let started = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish() }
        harness.stub { _ in
            started.continuation.yield(())
            throw URLError(.notConnectedToInternet)
        }
        let workspace = WorkspaceStore(client: NativeChatClient(api: harness.api, sync: sync), user: syncUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let running = Task { await workspace.run() }
        defer { running.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        #expect(workspace.hasLoaded)
        #expect(workspace.directoryReady == (pinned != nil))
        #expect(workspace.pinnedChannelIDs == (pinned == true ? [syncChannel] : []))
        #expect(workspace.channels == [channel])
        #expect(workspace.joinedChannelIDs == (joined ? [syncChannel] : []))
        #expect(workspace.unread == (joined ? [syncChannel] : []))
        running.cancel()
        await running.value
    }

    @Test("保存を省略する204応答でも待機中の権限失効後にはキャッシュを再配信しない")
    func unchangedMessageResponseRespectsRevocation() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "100_0"; state.shape.isLive = true
        state.shape.rows = ["k1": syncRow(1)]
        state.windows["root"] = MessageWindow(parentID: nil, limit: 50, hasMore: false)
        try await sync.store(state, key: "messages-v1:" + syncChannel, channelID: syncChannel)
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { _ in
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (204, ["electric-handle": "h", "electric-offset": "100_0"], Data())
        }
        var received = 0
        let observing = Task {
            defer { started.continuation.finish() }
            try await MessageSync(api: harness.api, sync: sync, channelID: syncChannel).observe(MessageFeedRequest()) { _ in received += 1 }
        }
        defer { observing.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        #expect(received == 1)
        try await sync.revokeChannel(syncChannel)
        release.continuation.finish()
        await #expect(throws: CancellationError.self) { try await observing.value }
        #expect(received == 1)
    }

    @Test("変化のない204応答では保存せず、カーソルだけの変更は保存する")
    func skipsUnchangedMessagePersistence() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let saves = Mutex(0)
        let cache = try await SyncCache.open(inMemory: true, beforeSave: { saves.withLock { $0 += 1 } })
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        let sync = SyncSession(cache: cache, lease: lease)
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "100_0"; state.shape.isLive = true
        state.shape.rows = ["k1": syncRow(1)]
        state.windows["root"] = MessageWindow(parentID: nil, limit: 50, hasMore: false)
        let key = "messages-v1:" + syncChannel
        try await sync.store(state, key: key, channelID: syncChannel)
        saves.withLock { $0 = 0 }
        let requests = Mutex(0)
        harness.stub { _ in
            let count = requests.withLock { $0 += 1; return $0 }
            if count == 4 { throw CancellationError() }
            if count <= 3 { #expect(saves.withLock { $0 } == 0) }
            var headers = ["electric-handle": "h", "electric-offset": "100_0"]
            if count == 3 { headers["electric-cursor"] = "次のカーソル" }
            return (204, headers, Data())
        }
        await #expect(throws: (any Error).self) {
            try await MessageSync(api: harness.api, sync: sync, channelID: syncChannel).observe(MessageFeedRequest()) { _ in }
        }
        #expect(requests.withLock { $0 } == 4)
        #expect(saves.withLock { $0 } == 1)
        #expect(try await sync.load(key, as: MessageSyncState.self)?.shape.cursor == "次のカーソル")
    }

    private func session(_ harness: ChatHarness) async throws -> SyncSession {
        let cache = try await SyncCache.open(inMemory: true)
        let lease = try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: syncUser)
        return SyncSession(cache: cache, lease: lease)
    }

    @Test("初回とライブの一覧はディスク保存が失敗しても返す", arguments: [false, true])
    func snapshotSurvivesPersistenceFailure(observe: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let fail = Mutex(false)
        let cache = try await SyncCache.open(inMemory: true, beforeSave: {
            if fail.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let lease = try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: syncUser)
        let sync = SyncSession(cache: cache, lease: lease)
        fail.withLock { $0 = true }
        harness.stub { _ in
            let events: [JSONValue] = [.object(["key": .string("k1"), "headers": .object(["operation": .string("insert")]), "value": .object(syncRow(1))])]
            return (200, ["electric-handle": "h", "electric-offset": "1_0", "electric-up-to-date": "true"], try JSONEncoder().encode(events))
        }
        let shapes = ShapeStream(api: harness.api, sync: sync)
        if observe {
            let received = AsyncStream<[Message]>.makeStream()
            let task = Task {
                defer { received.continuation.finish() }
                try await shapes.observe("api/shapes/messages", query: ["channel_id": syncChannel], as: Message.self) {
                    if let rows = $0 { received.continuation.yield(rows) }
                }
            }
            defer { task.cancel(); received.continuation.finish() }
            var iterator = received.stream.makeAsyncIterator()
            #expect(try #require(await iterator.next()).count == 1)
            #expect(try #require(await iterator.next()).count == 1)
            task.cancel()
            _ = await task.result
        } else {
            let values = try await shapes.snapshot("api/shapes/messages", query: ["channel_id": syncChannel], as: Message.self)
            #expect(values.count == 1)
        }
    }

    @Test("閉じたセッションの遅い失効通知はワークスペースへ届かない")
    func closedSessionDoesNotRevokeWorkspace() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var callbacks = 0
        sync.onChannelRevoked = { _, _ in callbacks += 1 }
        try await sync.close().value
        await #expect(throws: CancellationError.self) { try await sync.revokeChannel(syncChannel) }
        #expect(callbacks == 0)
        #expect(sync.knownChannels.isEmpty)
    }

    @Test("複数チャンネルの同時失効後も最新の通知世代で保存できる")
    func overlappingRevocationsKeepNotificationVersion() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let tasks = (0..<20).map { id in Task { try await sync.revokeChannel("channel-\(id)") } }
        for task in tasks { try await task.value }
        let version = try #require(sync.revocationVersion(for: "api/shapes/notifications"))
        try await sync.store([String](), key: "snapshot:api/shapes/notifications", revocationVersion: version)
        #expect(try await sync.load("snapshot:api/shapes/notifications", as: [String].self) == [])
    }

    @Test("再取得で保存したoffsetを使い、オフラインでは取得済みの行を返す")
    func resumesSnapshot() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let requests = Mutex<[String]>([])
        harness.stub { request in
            let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            requests.withLock { $0.append(query.first { $0.name == "offset" }?.value ?? "") }
            let count = requests.withLock { $0.count }
            if count == 3 { throw URLError(.notConnectedToInternet) }
            let events: [JSONValue] = count == 1
                ? [.object(["key": .string("k1"), "headers": .object(["operation": .string("insert")]), "value": .object(syncRow(1))])]
                : []
            return (200, ["electric-handle": "h", "electric-offset": "\(count)_0", "electric-up-to-date": "true"], try JSONEncoder().encode(events))
        }
        let stream = ShapeStream(api: harness.api, sync: sync)
        for _ in 0..<3 {
            #expect(try await stream.snapshot("api/shapes/messages", query: ["channel_id": syncChannel], as: Message.self).count == 1)
        }
        #expect(requests.withLock { $0 } == ["-1", "1_0", "2_0"])
    }

    @Test("差分取得の途中で切断しても最後に確定したスナップショットを残す")
    func preservesSnapshotDuringFailedCatchup() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var state = ShapeState()
        state.handle = "h"; state.offset = "100_0"; state.isLive = true
        state.rows = ["k1": syncRow(1)]
        try await sync.store(state, key: "snapshot:api/shapes/users")
        let calls = Mutex(0)
        harness.stub { _ in
            let count = calls.withLock { $0 += 1; return $0 }
            if count == 1 { return (200, ["electric-handle": "h", "electric-offset": "101_0"], Data("[]".utf8)) }
            throw URLError(.notConnectedToInternet)
        }
        for _ in 0..<2 {
            let messages = try await ShapeStream(api: harness.api, sync: sync).snapshot("api/shapes/users", as: Message.self)
            #expect(messages.count == 1)
        }
        let saved = try #require(await sync.load("snapshot:api/shapes/users", as: ShapeState.self))
        #expect(saved.isLive)
        #expect(saved.offset == "100_0")
    }

    @Test("オフラインで会話を繰り返し開いても通常文とMFMの履歴が残る", arguments: ["投稿1", "**太字** $[spin 回転] \\(x^2\\) :smile:"])
    func preservesOfflineMessages(content: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "100_0"; state.shape.isLive = true
        _ = try state.addSnapshot(subsetData([1]), response: syncResponse())
        state.shape.rows["k1"]?["content"] = .string(content)
        state.windows["root"] = MessageWindow(parentID: nil, limit: 50, hasMore: false)
        let key = "messages-v1:" + syncChannel
        try await sync.store(state, key: key, channelID: syncChannel)
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        for _ in 0..<2 {
            var snapshots: [MessageFeedSnapshot] = []
            await #expect(throws: URLError.self) {
                try await NativeChatClient(api: harness.api, sync: sync).observeMessageFeed(channelID: syncChannel,
                                                                                         request: MessageFeedRequest()) {
                    snapshots.append($0)
                }
            }
            #expect(snapshots.count == 1)
            #expect(snapshots.first?.fromCache == true)
            #expect(snapshots.first?.messages?.first?.content == content)
            #expect(try await sync.load(key, as: MessageSyncState.self)?.shape.isLive == true)
        }
    }

    @Test("キャッシュ失効の削除が失敗しても403と404で表示中の会話を消す", arguments: [403, 404])
    func clearsConversationDespiteCacheFailure(status: Int) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("test.store")
        let fails = Mutex(false)
        let cache = try await SyncCache.open(url: url, beforeDelete: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        let sync = SyncSession(cache: cache, lease: lease)
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "100_0"; state.shape.isLive = true
        _ = try state.addSnapshot(subsetData([1]), response: syncResponse())
        state.windows["root"] = MessageWindow(parentID: nil, limit: 50, hasMore: false)
        try await sync.store(state, key: "messages-v1:" + syncChannel, channelID: syncChannel)
        let directory = try #require(FileManager.default.contentsOfDirectory(
            at: url.appendingPathExtension("permissions"), includingPropertiesForKeys: nil).first)
        try FileManager.default.setAttributes([.posixPermissions: 0o500], ofItemAtPath: directory.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path) }
        fails.withLock { $0 = true }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { _ in
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (status, [:], Data())
        }
        let store = ConversationStore(channel: Channel(id: syncChannel, channel_name: "失効確認", is_private: true, is_dm: false),
                                      client: NativeChatClient(api: harness.api, sync: sync), userID: syncUser.id)
        let running = Task { defer { started.continuation.finish() }; await store.run() }
        defer { running.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        #expect(store.messages.first?.content == "投稿1")
        release.continuation.yield(())
        let deadline = ContinuousClock.now.advanced(by: .seconds(3))
        while !store.blocked && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(10)) }
        running.cancel()
        await running.value
        #expect(store.blocked && store.messages.isEmpty && !store.canPost)
    }

    @Test("再招待をサーバーで確認したら保存を再開し、失効前の応答は拒否する")
    func restoresChannelAccess() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let oldVersion = sync.channelVersion(syncChannel)
        try await sync.store("失効前", key: "messages", channelID: syncChannel, channelVersion: oldVersion)
        harness.stub { _ in (404, [:], Data()) }
        await #expect(throws: ZooError.self) {
            _ = try await NativeChatClient(api: harness.api, sync: sync).channel(id: syncChannel)
        }
        #expect(try await sync.load("messages", as: String.self) == nil)
        await #expect(throws: CancellationError.self) {
            try await sync.store("遅い応答", key: "messages", channelID: syncChannel, channelVersion: oldVersion)
        }
        harness.stub { request in
            #expect(request.url?.path == "/api/channels/" + syncChannel)
            return (200, [:], try JSONEncoder().encode(Channel(id: syncChannel, channel_name: "再招待", is_private: true, is_dm: false)))
        }
        _ = try await NativeChatClient(api: harness.api, sync: sync).channel(id: syncChannel)
        #expect(!sync.isRevoked(syncChannel))
        let newVersion = sync.channelVersion(syncChannel)
        try await sync.store("復旧後", key: "messages", channelID: syncChannel, channelVersion: newVersion)
        await #expect(throws: CancellationError.self) {
            try await sync.store("遅い応答", key: "messages", channelID: syncChannel, channelVersion: oldVersion)
        }
        await #expect(throws: CancellationError.self) {
            try await sync.cache.store(Data(), resource: "messages", channelID: syncChannel, channelVersion: oldVersion, lease: sync.lease)
        }
        #expect(try await sync.load("messages", as: String.self) == "復旧後")
    }

    @Test("権限失効前の一覧を保存・表示せず、内部エラーを出さずに次の一覧で再開する", arguments: [false, true])
    func rejectsLateWorkspaceDirectory(workspaceRefresh: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let workspace = WorkspaceStore(client: client, user: syncUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let channel = Channel(id: syncChannel, channel_name: "非公開の会話", is_private: true, is_dm: false)
        workspace.addAndOpen(channel)
        sync.onChannelRevoked = { [weak workspace] id, channel in workspace?.revokeChannel(id, retaining: channel) }
        try await sync.store([channel], key: "workspace:channels")
        try await sync.store([syncUser], key: "workspace:people")
        try await sync.store([ChannelReadState](), key: "rest:api/channel-read-states")
        #expect(try await client.cachedWorkspace()?.channels == [channel])
        let oldVersion = sync.revocationVersion(for: "workspace:channels")
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let row = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(channel))
        harness.stub { request in
            if request.url?.path == "/api/channels/" + syncChannel { return (403, [:], Data()) }
            if request.url?.path == "/api/channel-read-states" { return (200, [:], Data("[]".utf8)) }
            if request.url?.path == "/api/shapes/users" {
                return (200, ["electric-handle": "users", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            }
            #expect(request.url?.path == "/api/shapes/channels")
            started.continuation.yield(())
            for await _ in release.stream { break }
            let events: [JSONValue] = [.object(["key": .string("channel"), "headers": .object(["operation": .string("insert")]), "value": row])]
            return (200, ["electric-handle": "h", "electric-offset": "1_0", "electric-up-to-date": "true"], try JSONEncoder().encode(events))
        }
        let pending = Task {
            defer { started.continuation.finish() }
            if workspaceRefresh { await workspace.refresh(); return [Channel]() }
            return try await client.channels()
        }
        defer { pending.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        await #expect(throws: ZooError.self) { _ = try await client.channel(id: syncChannel) }
        #expect(workspace.channels.isEmpty && workspace.channelPath.isEmpty)
        #expect(try await client.cachedWorkspace() == nil)
        release.continuation.finish()
        if workspaceRefresh {
            _ = try await pending.value
            #expect(workspace.error == nil)
            #expect(workspace.channels.isEmpty)
            #expect(!workspace.loading)
        } else {
            await #expect(throws: SyncInvalidation.self) { _ = try await pending.value }
        }
        await #expect(throws: SyncInvalidation.self) {
            try await sync.storeRecovery([channel], key: "workspace:channels", revocationVersion: oldVersion)
        }
        await #expect(throws: SyncInvalidation.self) {
            try await sync.cache.store(JSONEncoder().encode([channel]), resource: "workspace:channels", revocationVersion: oldVersion, lease: sync.lease)
        }
        #expect(try await client.cachedWorkspace() == nil)
        #expect(try await sync.load("snapshot:api/shapes/channels", as: ShapeState.self) == nil)
        harness.stub { _ in (200, ["electric-handle": "fresh", "electric-offset": "2_0", "electric-up-to-date": "true"], Data("[]".utf8)) }
        #expect(try await client.channels().isEmpty)
        _ = try await client.readStates()
        #expect(try await client.cachedWorkspace()?.channels == [])
    }

    @Test("既読位置は旧形式を復元せず、取得後はオフライン復元し、権限失効で破棄する")
    func readStateCacheRequiresChannelPermission() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let path = "api/channels/" + syncChannel + "/read-state"
        let state = ChannelReadState(channel_id: syncChannel, has_unread: true, last_read_at: "2026-11-20T00:00:00Z")
        try await sync.store(state, key: "rest:" + path)
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        await #expect(throws: URLError.self) { _ = try await client.readState(channelID: syncChannel, allowCached: true) }
        harness.stub { _ in (200, [:], try JSONEncoder().encode(state)) }
        #expect(try await client.readState(channelID: syncChannel, allowCached: true).last_read_at == state.last_read_at)
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        #expect(try await client.readState(channelID: syncChannel, allowCached: true).has_unread)
        await #expect(throws: URLError.self) { _ = try await client.readState(channelID: syncChannel, allowCached: false) }
        harness.stub { _ in (403, [:], Data()) }
        await #expect(throws: ZooError.self) { _ = try await client.readState(channelID: syncChannel, allowCached: true) }
        #expect(sync.isRevoked(syncChannel))
        #expect(try await sync.load("channel-rest:" + path, as: ChannelReadState.self) == nil)
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        await #expect(throws: URLError.self) { _ = try await client.readState(channelID: syncChannel, allowCached: true) }
        let restarted = SyncSession(cache: sync.cache, lease: try await sync.cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: syncUser))
        let afterRestart = NativeChatClient(api: harness.api, sync: restarted)
        await #expect(throws: URLError.self) { _ = try await afterRestart.readState(channelID: syncChannel, allowCached: true) }
    }

    @Test("再招待後の最初の既読位置取得で権限を確認し直す", arguments: [false, true])
    func readStateReauthorizesReinvitedChannel(allowCached: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        try await sync.revokeChannel(syncChannel)
        var authorized: [String] = []
        sync.onChannelAuthorized = { authorized.append($0) }
        let paths = Mutex<[String]>([])
        harness.stub { request in
            let path = request.url?.path ?? ""
            paths.withLock { $0.append(path) }
            if path == "/api/channels/" + syncChannel {
                return (200, [:], try JSONEncoder().encode(Channel(id: syncChannel, channel_name: "再招待", is_private: true, is_dm: false)))
            }
            #expect(path == "/api/channels/" + syncChannel + "/read-state")
            return (200, [:], try JSONEncoder().encode(ChannelReadState(channel_id: syncChannel, has_unread: true, last_read_at: nil)))
        }
        let value = try await NativeChatClient(api: harness.api, sync: sync).readState(channelID: syncChannel, allowCached: allowCached)
        #expect(value.has_unread && value.channel_id == syncChannel)
        #expect(!sync.isRevoked(syncChannel) && authorized == [syncChannel])
        #expect(paths.withLock { $0 } == ["/api/channels/" + syncChannel, "/api/channels/" + syncChannel + "/read-state"])
    }

    @Test("既読位置の取得中に権限が失効したら成功応答もオフライン復元も返さない", arguments: [false, true], [false, true])
    func rejectsLateReadState(allowCached: Bool, offline: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let path = "api/channels/" + syncChannel + "/read-state"
        let state = ChannelReadState(channel_id: syncChannel, has_unread: true, last_read_at: "2026-11-20T00:00:00Z")
        harness.stub { _ in (200, [:], try JSONEncoder().encode(state)) }
        _ = try await client.readState(channelID: syncChannel, allowCached: true)
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            if request.url?.path == "/api/channels/" + syncChannel { return (403, [:], Data()) }
            #expect(request.url?.path == "/" + path)
            started.continuation.yield(())
            for await _ in release.stream { break }
            if offline { throw URLError(.notConnectedToInternet) }
            return (200, [:], try JSONEncoder().encode(state))
        }
        let pending = Task {
            defer { started.continuation.finish() }
            return try await client.readState(channelID: syncChannel, allowCached: allowCached)
        }
        defer { pending.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        await #expect(throws: ZooError.self) { _ = try await client.channel(id: syncChannel) }
        release.continuation.finish()
        await #expect(throws: CancellationError.self) { _ = try await pending.value }
        #expect(try await sync.load("channel-rest:" + path, as: ChannelReadState.self) == nil)
    }

    @Test("既読一覧とピン一覧は失効前の応答を保存せず、オフラインでも旧一覧を返さない",
          arguments: ["api/channel-read-states", "api/channel-pins"])
    func rejectsLateRESTDirectory(path: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let value = JSONValue.array([.string(syncChannel)])
        try await sync.store(value, key: "rest:" + path)
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            #expect(request.url?.path == "/" + path)
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, [:], try JSONEncoder().encode(value))
        }
        let pending = Task {
            defer { started.continuation.finish() }
            return try await client.cachedGet(path) as JSONValue
        }
        defer { pending.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        try await sync.revokeChannel(syncChannel)
        release.continuation.finish()
        await #expect(throws: SyncInvalidation.self) { _ = try await pending.value }
        #expect(try await sync.load("rest:" + path, as: JSONValue.self) == nil)
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        await #expect(throws: URLError.self) { _ = try await client.cachedGet(path) as JSONValue }
        harness.stub { _ in (200, [:], Data("[]".utf8)) }
        #expect(try await client.cachedGet(path) as JSONValue == .array([]))
    }

    @Test("暗号化した会話の単件取得は画面の世代を変えず平文キャッシュを残さない")
    func encryptedChannelKeepsAccessWhilePurgingCache() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let version = sync.channelVersion(syncChannel)
        let key = "messages-v1:" + syncChannel
        try await sync.store("秘密になる前の本文", key: key, channelID: syncChannel, channelVersion: version)
        let channel = Channel(id: syncChannel, channel_name: "暗号化", is_private: true, is_dm: false, encrypted: true)
        harness.stub { _ in (200, [:], try JSONEncoder().encode(channel)) }
        var revoked = false
        sync.onChannelRevoked = { _, _ in revoked = true }
        let client = NativeChatClient(api: harness.api, sync: sync)
        #expect(try await client.channel(id: syncChannel) == channel)
        #expect(!revoked && !sync.isRevoked(syncChannel) && sync.channelVersion(syncChannel) == version)
        try sync.check(channelID: syncChannel, channelVersion: version)
        #expect(try await sync.load(key, as: String.self) == nil)
        try await sync.store("古い応答", key: key, channelID: syncChannel, channelVersion: version)
        #expect(try await sync.load(key, as: String.self) == nil)
    }

    @Test("削除は旧画面を閉じ、暗号化への切替は権限と画面を維持してキャッシュを消す", arguments: [false, true])
    func storesDirectoryAfterOwnRevocations(encrypted: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let initial = Channel(id: syncChannel, channel_name: "変更前", is_private: true, is_dm: false)
        try await sync.store([initial], key: "workspace:channels")
        try await sync.store([syncUser], key: "workspace:people")
        var protected = initial
        protected.encrypted = true
        let expected = encrypted ? [protected] : []
        let row = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(protected))
        let events: [JSONValue] = encrypted
            ? [.object(["key": .string("channel"), "headers": .object(["operation": .string("insert")]), "value": row])] : []
        harness.stub { request in
            if request.url?.path == "/api/channels" { return (200, [:], try JSONEncoder().encode(expected)) }
            let bytes = request.url?.path == "/api/shapes/channels" ? try JSONEncoder().encode(events) : Data("[]".utf8)
            return (200, ["electric-handle": "fresh", "electric-offset": "2_0", "electric-up-to-date": "true"], bytes)
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        let workspace = WorkspaceStore(client: client, user: syncUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        workspace.addAndOpen(initial)
        sync.onChannelRevoked = { [weak workspace] id, channel in workspace?.revokeChannel(id, retaining: channel) }
        await workspace.refresh()
        #expect(workspace.channels == expected)
        #expect(workspace.channelPath.isEmpty == !encrypted)
        #expect(sync.isRevoked(syncChannel) == !encrypted)
        #expect(try await client.cachedWorkspace()?.channels == expected)
        await workspace.refresh()
        #expect(workspace.hasLoaded && workspace.error == nil)
        #expect(workspace.channels == expected)
    }

    @Test("暗号化済み会話が一覧から消えるか削除された場合も通話の権限を失効させる", arguments: [false, true])
    func revokesMissingEncryptedDirectoryChannel(deleted: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var initial = Channel(id: syncChannel, channel_name: "暗号化済み", is_private: true, is_dm: false)
        initial.encrypted = true
        try await sync.store([initial], key: "workspace:channels")
        var removed = initial
        removed.deleted_at = "2026-11-23 10:00:00"
        let row = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(removed))
        let events: [JSONValue] = deleted
            ? [.object(["key": .string("channel"), "headers": .object(["operation": .string("insert")]), "value": row])] : []
        harness.stub { request in
            if request.url?.path == "/api/channels" { return (200, [:], Data("[]".utf8)) }
            return (200, ["electric-handle": "fresh", "electric-offset": "2_0", "electric-up-to-date": "true"],
                    try JSONEncoder().encode(events))
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        let workspace = WorkspaceStore(client: client, user: syncUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        workspace.addAndOpen(initial)
        sync.onChannelRevoked = { [weak workspace] id, channel in workspace?.revokeChannel(id, retaining: channel) }
        #expect(!workspace.calls.isChannelRevoked(syncChannel))
        _ = try await client.channels()
        #expect(sync.isRevoked(syncChannel))
        #expect(workspace.channels.isEmpty && workspace.channelPath.isEmpty)
        #expect(workspace.calls.isChannelRevoked(syncChannel))
    }

    @Test("未保存の単件取得・保存失敗した一覧の会話も、確定一覧から消えたら失効させる",
          arguments: [false, true], [false, true])
    func revokesUnsavedKnownChannel(single: Bool, encrypted: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let fail = Mutex(false)
        let cache = try await SyncCache.open(inMemory: true, beforeSave: {
            if fail.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let sync = SyncSession(cache: cache, lease: try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: syncUser))
        let client = NativeChatClient(api: harness.api, sync: sync)
        let workspace = WorkspaceStore(client: client, user: syncUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        sync.onChannelRevoked = { [weak workspace] id, channel in workspace?.revokeChannel(id, retaining: channel) }
        var channel = Channel(id: syncChannel, channel_name: "未保存の会話", is_private: true, is_dm: false)
        channel.encrypted = encrypted
        let channelData = try JSONEncoder().encode(channel)
        let row = try JSONDecoder().decode(JSONValue.self, from: channelData)
        let events: [JSONValue] = [.object(["key": .string("channel"), "headers": .object(["operation": .string("insert")]), "value": row])]
        harness.stub { request in
            let bytes = request.url?.path == "/api/shapes/channels" ? try JSONEncoder().encode(events) : channelData
            return (200, ["electric-handle": "fresh", "electric-offset": "1_0", "electric-up-to-date": "true"], bytes)
        }
        if single { _ = try await client.channel(id: syncChannel) }
        else {
            fail.withLock { $0 = true }
            #expect(try await client.channels() == [channel])
            fail.withLock { $0 = false }
        }
        workspace.addAndOpen(channel)
        #expect(try await sync.load("workspace:channels", as: [Channel].self) == nil)
        #expect(!workspace.calls.isChannelRevoked(syncChannel))
        harness.stub { _ in
            (200, ["electric-handle": "empty", "electric-offset": "2_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        #expect(try await client.channels().isEmpty)
        #expect(workspace.channels.isEmpty && workspace.channelPath.isEmpty)
        #expect(sync.isRevoked(syncChannel) && workspace.calls.isChannelRevoked(syncChannel))
    }

    @Test("別会話の失効で永続一覧が消えても、復元済みの会話の消失を検出する")
    func remembersRestoredChannelsAfterCacheInvalidation() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let channel = Channel(id: syncChannel, channel_name: "復元した会話", is_private: true, is_dm: false)
        try await sync.store([channel], key: "workspace:channels")
        try await sync.store([syncUser], key: "workspace:people")
        try await sync.store([ChannelReadState](), key: "rest:api/channel-read-states")
        let client = NativeChatClient(api: harness.api, sync: sync)
        #expect(try await client.cachedWorkspace()?.channels == [channel])
        try await sync.revokeChannel("別の会話")
        #expect(try await sync.load("workspace:channels", as: [Channel].self) == nil)
        harness.stub { _ in
            (200, ["electric-handle": "empty", "electric-offset": "2_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        #expect(try await client.channels().isEmpty)
        #expect(sync.isRevoked(syncChannel))
    }

    @Test("単件取得後は古い一覧とオフライン復元で会話・下書き・通話を失効させない")
    func rejectsDirectoryOlderThanSingleChannel() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let channel = Channel(id: syncChannel, channel_name: "新たな会話", is_private: true, is_dm: false)
        let callClient = FakeCallClient(), media = FakeCallMedia()
        callClient.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let calls = CallStore(client: callClient, userID: syncUser.id, makeMedia: { media }, permission: { true })
        let workspace = WorkspaceStore(client: client, user: syncUser, recentEmojiDefaults: nil, calls: calls)
        defer { workspace.shutdown() }
        sync.onChannelRevoked = { [weak workspace] id, channel in workspace?.revokeChannel(id, retaining: channel) }
        let bytes = try JSONEncoder().encode(channel)
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            if request.url?.path == "/api/channels/" + syncChannel { return (200, [:], bytes) }
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, ["electric-handle": "empty", "electric-offset": "2_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        let pending = Task {
            defer { started.continuation.finish() }
            return try await client.channels()
        }
        defer { pending.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        #expect(try await client.channel(id: syncChannel) == channel)
        workspace.addAndOpen(channel)
        let conversation = workspace.conversation(for: channel)
        conversation.drafts["root"] = "保存する下書き"
        try await waitForCallDelivery(callClient) { calls.start(channel: channel) }
        release.continuation.finish()
        await #expect(throws: SyncInvalidation.self) { _ = try await pending.value }
        #expect(!sync.isRevoked(syncChannel))
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        await #expect(throws: URLError.self) { _ = try await client.channels() }
        #expect(!sync.isRevoked(syncChannel) && calls.ready && !media.closed)
        #expect(workspace.channels == [channel] && conversation.drafts["root"] == "保存する下書き")
        harness.stub { _ in
            (200, ["electric-handle": "empty", "electric-offset": "3_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        #expect(try await client.channels().isEmpty)
        #expect(sync.isRevoked(syncChannel))
        #expect(calls.channel == nil && media.closed && conversation.drafts.isEmpty)
    }

    @Test("単件取得後に遅れたShapeが来ても正本を一度照合し、確認済みの失効だけを反映する",
          arguments: ["許可", "失効", "通信断"])
    func confirmsDirectoryRevocation(confirmation: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let channel = Channel(id: syncChannel, channel_name: "参加直後", is_private: true, is_dm: false)
        let other = Channel(id: "22222222-2222-4222-8222-222222222222", channel_name: "もう一つの会話", is_private: true, is_dm: false)
        let rows = [channel, other]
        harness.stub { request in
            let row = request.url?.path.hasSuffix(syncChannel) == true ? channel : other
            return (200, [:], try JSONEncoder().encode(row))
        }
        _ = try await client.channel(id: channel.id)
        _ = try await client.channel(id: other.id)
        let callClient = FakeCallClient(), media = FakeCallMedia()
        callClient.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let calls = CallStore(client: callClient, userID: syncUser.id, makeMedia: { media }, permission: { true })
        let workspace = WorkspaceStore(client: client, user: syncUser, recentEmojiDefaults: nil, calls: calls)
        defer { workspace.shutdown() }
        sync.onChannelRevoked = { [weak workspace] id, channel in workspace?.revokeChannel(id, retaining: channel) }
        workspace.addAndOpen(channel)
        let conversation = workspace.conversation(for: channel)
        conversation.drafts["root"] = "保持する下書き"
        try await waitForCallDelivery(callClient) { calls.start(channel: channel) }
        let requests = Mutex<[String]>([])
        harness.stub { request in
            let path = request.url?.path ?? ""
            requests.withLock { $0.append(path) }
            if path == "/api/channels" {
                if confirmation == "通信断" { throw URLError(.notConnectedToInternet) }
                return (200, [:], try JSONEncoder().encode(confirmation == "許可" ? rows : []))
            }
            #expect(path == "/api/shapes/channels")
            return (200, ["electric-handle": "lagging", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        if confirmation == "通信断" {
            await #expect(throws: URLError.self) { _ = try await client.channels() }
        } else { #expect(try await client.channels() == (confirmation == "許可" ? rows : [])) }
        #expect(requests.withLock { $0 } == ["/api/shapes/channels", "/api/channels"])
        if confirmation == "失効" {
            #expect(sync.isRevoked(channel.id) && sync.isRevoked(other.id))
            #expect(!calls.ready && media.closed && conversation.drafts.isEmpty)
        } else {
            #expect(!sync.isRevoked(channel.id) && !sync.isRevoked(other.id))
            #expect(calls.ready && !media.closed && conversation.drafts["root"] == "保持する下書き")
        }
        let events = try rows.map { row -> JSONValue in
            .object(["key": .string(row.id), "headers": .object(["operation": .string("insert")]),
                     "value": try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(row))])
        }
        harness.stub { request in
            if request.url?.path == "/api/channels" { return (200, [:], try JSONEncoder().encode(rows)) }
            #expect(request.url?.path == "/api/shapes/channels")
            return (200, ["electric-handle": "caught-up", "electric-offset": "2_0", "electric-up-to-date": "true"], try JSONEncoder().encode(events))
        }
        #expect(Set(try await client.channels().map(\.id)) == Set(rows.map(\.id)))
    }

    @Test("遅れた暗号化Shapeでは失効を解除せず、単件RESTの再認可だけで復帰する")
    func encryptedShapeDoesNotAuthorizeRevokedChannel() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let channel = Channel(id: syncChannel, channel_name: "失効済み", is_private: true, is_dm: false, encrypted: true)
        _ = try await sync.revokeChannel(syncChannel)
        var authorizations = 0
        sync.onChannelAuthorized = { _ in authorizations += 1 }
        _ = try await sync.excludeFromCache(syncChannel, version: sync.channelVersion(syncChannel))
        #expect(sync.isRevoked(syncChannel) && authorizations == 0)
        let event = JSONValue.object(["key": .string(syncChannel), "headers": .object(["operation": .string("insert")]),
            "value": try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(channel))])
        harness.stub { request in
            if request.url?.path == "/api/channels" { return (200, [:], Data("[]".utf8)) }
            return (200, ["electric-handle": "old", "electric-offset": "1_0", "electric-up-to-date": "true"], try JSONEncoder().encode([event]))
        }
        #expect(try await client.channels().isEmpty)
        #expect(sync.isRevoked(syncChannel) && authorizations == 0)
        harness.stub { _ in (200, [:], try JSONEncoder().encode(channel)) }
        #expect(try await client.channel(id: syncChannel) == channel)
        #expect(!sync.isRevoked(syncChannel) && authorizations == 1)
        #expect(try await sync.cache.load("channel-rest:api/channels/" + syncChannel, lease: sync.lease) == nil)
    }

    @Test("正本一覧の照合中に失効した会話を成功応答で復活させない")
    func rejectsLateAuthoritativeDirectory() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let client = NativeChatClient(api: harness.api, sync: sync)
        let channel = Channel(id: syncChannel, channel_name: "照合対象", is_private: true, is_dm: false)
        harness.stub { _ in (200, [:], try JSONEncoder().encode(channel)) }
        _ = try await client.channel(id: syncChannel)
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            if request.url?.path == "/api/channels" {
                started.continuation.yield(())
                for await _ in release.stream { break }
                return (200, [:], try JSONEncoder().encode([channel]))
            }
            return (200, ["electric-handle": "lagging", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        let pending = Task {
            defer { started.continuation.finish() }
            return try await client.channels()
        }
        defer { pending.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        try await sync.revokeChannel(syncChannel)
        release.continuation.finish()
        await #expect(throws: SyncInvalidation.self) { _ = try await pending.value }
        #expect(sync.isRevoked(syncChannel) && sync.knownChannels[syncChannel] == nil)
    }

    @Test("会話取得中の失効やログアウト後に古い成功応答を返さない", arguments: [false, true])
    func rejectsLateChannel(logout: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        harness.stub { _ in
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, [:], try JSONEncoder().encode(Channel(id: syncChannel, channel_name: "失効前", is_private: true, is_dm: false)))
        }
        let pending = Task {
            defer { started.continuation.finish() }
            return try await NativeChatClient(api: harness.api, sync: sync).channel(id: syncChannel)
        }
        defer { pending.cancel(); release.continuation.finish(); started.continuation.finish() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        if logout { try await sync.close().value }
        else { try await sync.revokeChannel(syncChannel) }
        release.continuation.yield(())
        await #expect(throws: CancellationError.self) { _ = try await pending.value }
    }

    @Test("権限喪失前の通知応答で本文を再保存・再表示しない", arguments: [false, true])
    func rejectsLateNotifications(observe: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        let notices = Data("""
        [{"key":"n","headers":{"operation":"insert"},"value":{"id":"n","user_id":"\(syncUser.id)","actor_id":"a","channel_id":"\(syncChannel)","message_id":null,"notification_type":"mention","created_at":"2026-11-19 10:00:00","is_read":false,"display_data":{"message":{"content":"削除する本文"}}}},{"headers":{"control":"up-to-date"}}]
        """.utf8)
        harness.stub { request in
            if request.url?.path == "/api/activitypub/notifications" {
                return (200, [:], Data(#"{"items":[],"unread_count":0,"next_cursor":null}"#.utf8))
            }
            return (200, ["electric-handle": "h", "electric-offset": "100_0"], notices)
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        let inbox = InboxStore(client: client, userID: syncUser.id)
        sync.onChannelRevoked = { [weak inbox] id, _ in inbox?.revokeChannel(id) }
        await inbox.refresh()
        #expect(inbox.entries.count == 1)
        let oldVersion = sync.revocationVersion(for: "api/shapes/notifications")
        harness.stub { _ in
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, ["electric-handle": "h", "electric-offset": "101_0"], notices)
        }
        var published = false
        let pending = Task {
            defer { started.continuation.finish() }
            if observe { try await client.observeInbox { _ in published = true } }
            else { _ = try await client.channelNoticeSnapshot(); published = true }
        }
        defer { pending.cancel(); release.continuation.finish(); started.continuation.finish() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        try await sync.revokeChannel(syncChannel)
        #expect(inbox.entries.isEmpty)
        release.continuation.yield(())
        await #expect(throws: SyncInvalidation.self) { try await pending.value }
        #expect(!published)
        for key in ["shape:api/shapes/notifications", "snapshot:api/shapes/notifications"] {
            #expect(try await sync.load(key, as: ShapeState.self) == nil)
            await #expect(throws: SyncInvalidation.self) {
                try await sync.cache.store(Data(), resource: key, revocationVersion: oldVersion, lease: sync.lease)
            }
        }
    }

    @Test("SSEの拒否からロングポーリングへ戻り、認証切れ・権限喪失は直ちに伝える",
          arguments: [401, 403, 404, 405, 429, 503], [false, true])
    func rejectedSSE(status: Int, messages: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "100_0"; state.shape.isLive = true
        state.windows["root"] = MessageWindow(parentID: nil, limit: 50, hasMore: false)
        if messages { try await sync.store(state, key: "messages-v1:" + syncChannel, channelID: syncChannel) }
        else { try await sync.store(state.shape, key: "shape:api/shapes/messages|channel_id=" + syncChannel, channelID: syncChannel) }
        let requests = Mutex((sse: 0, plain: 0))
        harness.stub { request in
            if request.value(forHTTPHeaderField: "Accept") == "text/event-stream" {
                requests.withLock { $0.sse += 1 }
                return (status, [:], Data())
            }
            let count = requests.withLock { $0.plain += 1; return $0.plain }
            if !messages || count > 1 { throw URLError(.cancelled) }
            return (200, ["electric-handle": "h", "electric-offset": "100_0", "electric-up-to-date": "true"],
                    Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
        }
        let fallback = ![401, 403, 404].contains(status)
        do {
            if messages {
                try await MessageSync(api: harness.api, sync: sync, channelID: syncChannel).observe(MessageFeedRequest()) { _ in }
            } else {
                try await ShapeStream(api: harness.api, sync: sync).observe("api/shapes/messages", query: ["channel_id": syncChannel], as: Message.self) { _ in }
            }
            Issue.record("終了させた同期が正常完了しました。")
        } catch ZooError.expiredSession { #expect(status == 401) }
        catch ZooError.rejected(let code, _) { #expect(code == status && [403, 404].contains(status)) }
        catch is CancellationError { #expect(fallback) }
        catch let error as URLError { #expect(fallback && error.code == .cancelled) }
        let counts = requests.withLock { $0 }
        #expect(counts.sse == (fallback ? 3 : 1))
        #expect(counts.plain == (messages ? 1 : 0) + (fallback ? 1 : 0))
        if status == 403 || status == 404 {
            #expect(throws: CancellationError.self) { try sync.check(channelID: syncChannel) }
        }
    }

    @Test("SSEはup-to-dateまでまとめ、未完了の末尾を公開しない")
    func sseBatch() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.value(forHTTPHeaderField: "Accept") == "text/event-stream")
            let data = """
            data: {"key":"k1","headers":{"operation":"insert"},"value":{"id":"1"}}

            data: {"headers":{"control":"up-to-date","global_last_seen_lsn":"123"}}

            data: {"key":"k2","headers":{"operation":"insert"},"value":{"id":"2"}}


            """
            return (200, ["Content-Type": "text/event-stream", "electric-handle": "h"], Data(data.utf8))
        }
        var batches = 0
        do {
            _ = try await ElectricTransport(api: harness.api).read("api/shapes/messages", query: ["live": "true"], sse: true) { data, response in
            batches += 1
            #expect(response.value(forHTTPHeaderField: "electric-offset") == "123_0")
            let events = try JSONDecoder().decode([ShapeEvent].self, from: data)
            #expect(events.count == 2)
        }
            Issue.record("同期途中の EOF が受け入れられました。")
        } catch ZooError.invalidResponse { }
        #expect(batches == 1)
    }

    private func feed(_ client: NativeChatClient, request: MessageFeedRequest) async throws -> MessageFeedSnapshot {
        let updates = AsyncStream<MessageFeedSnapshot>.makeStream()
        let task = Task {
            defer { updates.continuation.finish() }
            try await client.observeMessageFeed(channelID: syncChannel, request: request) { updates.continuation.yield($0) }
        }
        return try await withTaskCancellationHandler {
            for await value in updates.stream where !value.fromCache && value.messages != nil {
                task.cancel()
                _ = await task.result
                return value
            }
            try await task.value
            throw ZooError.invalidResponse
        } onCancel: { task.cancel() }
    }

    @Test("暗号化フィードは既存の平文キャッシュを読まず、新しい行も保存しない")
    func encryptedFeedBypassesCache() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var cached = MessageSyncState()
        cached.shape.handle = "h"; cached.shape.offset = "100_0"; cached.shape.isLive = true
        _ = try cached.addSnapshot(subsetData([1]), response: syncResponse())
        let key = "messages-v1:" + syncChannel
        try await sync.store(cached, key: key, channelID: syncChannel)
        let original = try #require(await sync.cache.load(key, lease: sync.lease))
        harness.stub { request in
            let url = try #require(request.url)
            let components = try #require(URLComponents(string: url.absoluteString))
            let query = try #require(components.queryItems)
            #expect(!query.contains { $0.name == "handle" && $0.value == "h" })
            if query.contains(where: { $0.name == "subset__where" }) {
                let data = try subsetData([2])
                return (200, ["electric-handle": "fresh", "electric-offset": "100_0"], data)
            }
            return (200, ["electric-handle": "fresh", "electric-offset": "100_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        let updates = AsyncStream<MessageFeedSnapshot>.makeStream()
        let task = Task {
            defer { updates.continuation.finish() }
            try await NativeChatClient(api: harness.api, sync: sync).observeMessageFeed(
                channelID: syncChannel, request: MessageFeedRequest(encrypted: true)) { updates.continuation.yield($0) }
        }
        defer { task.cancel() }
        let receivedFresh = try await withTaskCancellationHandler {
            for await snapshot in updates.stream {
                #expect(!snapshot.fromCache)
                #expect(snapshot.messages?.contains { $0.content == "投稿1" } != true)
                if snapshot.messages?.contains(where: { $0.content == "投稿2" }) == true { return true }
            }
            try await task.value
            return false
        } onCancel: { task.cancel() }
        task.cancel(); _ = await task.result
        try #require(receivedFresh)
        #expect(try await sync.cache.load(key, lease: sync.lease) == original)
    }

    @Test("最初の50件、次の50件、保存位置からの再開で全履歴を取り直さない", arguments: [false, true])
    func pagesAndResumption(encrypted: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let requests = Mutex<[[String: String]]>([])
        harness.stub { request in
            let query = Dictionary((URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []).map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { _, new in new })
            requests.withLock { $0.append(query) }
            if let predicate = query["subset__where"] {
                #expect(query["subset__limit"] == "50")
                if predicate.contains("created_at <") {
                    let parameters = try JSONDecoder().decode([String: String].self, from: Data((query["subset__params"] ?? "").utf8))
                    #expect(parameters.keys.sorted() == ["1", "2"])
                    #expect(parameters["1"] == syncRow(51)["created_at"]?.text)
                    #expect(parameters["2"] == syncRow(51)["id"]?.text)
                    #expect(predicate.contains("created_at < $1"))
                    #expect(predicate.contains("id < $2"))
                }
                let numbers = predicate.contains("created_at <") ? Array((1...50).reversed()) : Array((51...100).reversed())
                return (200, ["electric-handle": "h", "electric-offset": "100_0"], try subsetData(numbers))
            }
            return (200, ["electric-handle": "h", "electric-offset": "100_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        let memory = MessageFeedMemory()
        #expect(try await feed(client, request: MessageFeedRequest(encrypted: encrypted, memory: memory)).messages?.count == 50)
        #expect(try await feed(client, request: MessageFeedRequest(limit: 100, encrypted: encrypted, memory: memory)).messages?.count == 100)
        #expect(try await feed(client, request: MessageFeedRequest(limit: 100, encrypted: encrypted, memory: memory)).messages?.count == 100)
        let values = requests.withLock { $0 }
        #expect(values.filter { $0["subset__where"] != nil }.count == 2)
        #expect(values.filter { $0["offset"] == "now" }.count == 1)
        #expect(!values.contains { $0["offset"] == "-1" })
        if encrypted { #expect(try await sync.cache.load("messages-v1:" + syncChannel, lease: sync.lease) == nil) }
    }

    @Test("暗号化履歴500件は10回で読み、一覧へ戻った後は最初の50件から再開する", arguments: [false, true])
    func encryptedHistoryUsesOnlyNewPages(directMessage: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let subsets = Mutex(0)
        let ready = AsyncStream<Bool>.makeStream()
        defer { ready.continuation.finish() }
        harness.stub { request in
            let query = Dictionary((URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? [])
                .map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { _, new in new })
            if query["subset__where"] != nil {
                subsets.withLock { $0 += 1 }
                let parameters = try JSONDecoder().decode([String: String].self, from: Data((query["subset__params"] ?? "{}").utf8))
                let last = parameters["2"].flatMap { Int($0.suffix(12)) } ?? 501
                return (200, ["electric-handle": "h", "electric-offset": "100_0"], try subsetData(Array((last - 50)..<last)))
            }
            if query["live"] == "true" {
                ready.continuation.yield(true)
                try await Task.sleep(for: .seconds(60))
            }
            return (200, ["electric-handle": "h", "electric-offset": "100_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        let workspace = WorkspaceStore(client: client, user: syncUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        workspace.e2ee = E2eeController.makePreview(userID: syncUser.id)
        let channel = Channel(id: syncChannel, channel_name: "暗号化履歴", is_private: true, is_dm: directMessage, encrypted: true)
        workspace.addAndOpen(channel)
        let store = workspace.conversation(for: channel)
        store.drafts["root"] = "保持する下書き"
        func start() -> Task<Void, Never> {
            Task {
                defer { ready.continuation.yield(false) }
                await store.run()
            }
        }
        var run = start()
        defer { run.cancel() }
        var iterator = ready.stream.makeAsyncIterator()
        for page in 1...10 {
            if page > 1 {
                store.loadOlder(parentID: nil)
                run.cancel(); await run.value
                try #require(await iterator.next() == false)
                run = start()
            }
            try #require(await iterator.next() == true)
            #expect(store.messages.count == page * 50)
            #expect(subsets.withLock { $0 } == page)
        }
        #expect(try await sync.cache.load("messages-v1:" + syncChannel, lease: sync.lease) == nil)
        let thread = ChatRoute.thread(channelID: channel.id, parentID: syncRow(500)["id"]!.text!, focusComposer: false)
        if directMessage { workspace.dmPath.append(thread); workspace.dmPath.removeLast() }
        else { workspace.channelPath.append(thread); workspace.channelPath.removeLast() }
        #expect(store.messages.count == 500)
        if directMessage { workspace.dmPath = [] } else { workspace.channelPath = [] }
        #expect(store.messages.isEmpty && !store.hasOlder(parentID: nil))
        #expect(store.drafts["root"] == "保持する下書き")
        run.cancel(); await run.value
        try #require(await iterator.next() == false)
        workspace.addAndOpen(channel)
        #expect(workspace.conversation(for: channel) === store)
        run = start()
        try #require(await iterator.next() == true)
        #expect(store.messages.count == 50)
        #expect(subsets.withLock { $0 } == 11)
    }

    @Test("保存済みの100件に続く50件を要求し、オフライン・重複・別スレッドの追加取得を止める")
    func storePaginationResumesLoadedWindow() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "100_0"; state.shape.isLive = true
        _ = try state.addSnapshot(subsetData(Array(51...150)), response: syncResponse())
        state.windows["root"] = MessageWindow(parentID: nil, limit: 100, hasMore: true,
                                               oldestDate: "2026-11-19 10:00:00", oldestID: syncRow(51)["id"]?.text)
        try await sync.store(state, key: "messages-v1:" + syncChannel, channelID: syncChannel)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        let ready = AsyncStream<Bool>.makeStream()
        let firstRequest = Mutex(true)
        let subsets = Mutex(0)
        harness.stub { request in
            let query = Dictionary((URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []).map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { _, new in new })
            if firstRequest.withLock({ value in defer { value = false }; return value }) {
                started.continuation.yield(())
                for await _ in release.stream { break }
            }
            if query["subset__where"] != nil {
                subsets.withLock { $0 += 1 }
                return (200, ["electric-handle": "h", "electric-offset": "100_0"], try subsetData(Array(1...50)))
            }
            if query["live"] == "true" {
                ready.continuation.yield(true)
                try await Task.sleep(for: .seconds(60))
            }
            return (200, ["electric-handle": "h", "electric-offset": "100_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        let store = ConversationStore(channel: Channel(id: syncChannel, channel_name: "履歴", is_private: false, is_dm: false),
                                      client: NativeChatClient(api: harness.api, sync: sync), userID: syncUser.id)
        #expect(store.postingRestrictionMessage == nil)
        var run = Task {
            defer { started.continuation.finish(); ready.continuation.yield(false) }
            await store.run()
        }
        defer { run.cancel() }
        var starts = started.stream.makeAsyncIterator()
        try #require(await starts.next() != nil)
        #expect(store.showingCachedMessages)
        store.drafts["root"] = "同期中に入力した下書き"
        #expect(!store.canSend(parentID: nil))
        #expect(store.postingRestrictionMessage == nil)
        var channel = store.channel
        channel.archive_at = "2026-11-20T00:00:00Z"
        store.update(channel)
        #expect(store.postingRestrictionMessage == "アーカイブ済みのチャンネルです")
        channel.archive_at = nil
        store.update(channel)
        #expect(store.postingRestrictionMessage == nil)
        let initialRequest = store.syncRequest
        store.loadOlder(parentID: nil)
        #expect(store.syncRequest == initialRequest)
        release.continuation.yield(())
        var iterator = ready.stream.makeAsyncIterator()
        try #require(await iterator.next() == true)
        #expect(!store.showingCachedMessages && store.messages.count == 100)
        #expect(store.drafts["root"] == "同期中に入力した下書き")
        #expect(store.canSend(parentID: nil))
        #expect(store.postingRestrictionMessage == nil)
        #expect(store.historyRevision == 0)
        store.loadOlder(parentID: "別スレッド")
        #expect(store.syncRequest == initialRequest)
        store.loadOlder(parentID: nil)
        #expect(store.loadingOlder && store.syncRequest != initialRequest)
        let nextRequest = store.syncRequest
        store.loadOlder(parentID: nil)
        #expect(store.syncRequest == nextRequest)
        run.cancel()
        await run.value
        #expect(await iterator.next() == false)
        run = Task {
            defer { started.continuation.finish(); ready.continuation.yield(false) }
            await store.run()
        }
        try #require(await iterator.next() == true)
        #expect(store.messages.count == 150)
        #expect(store.historyRevision == 1)
        #expect(!store.loadingOlder)
        #expect(subsets.withLock { $0 } == 1)
    }

    @Test("同じアカウントの復元でも現在の管理権限とプロフィールを保存する", arguments: [false, true])
    func restoredProfileReflectsCurrentUser(wasAdmin: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let cache = try await SyncCache.open(inMemory: true)
        let connection = SyncCache.connectionKey(harness.api.settings)
        var previous = syncUser
        previous.is_admin = wasAdmin
        _ = try await cache.activate(connection: connection, user: previous)
        var latest = previous
        latest.is_admin = !wasAdmin
        latest.display_name = "現在の表示名"
        let response = try JSONEncoder().encode(latest)
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            #expect(request.url?.path == "/api/users/me")
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, [:], response)
        }
        let app = AppSession(settings: harness.api.settings, transport: harness.api.transport,
            notificationStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys(), syncCache: cache, voipStore: PreviewPushReceiptStore())
        defer { app.logout() }
        let pending = Task { await app.resume(); started.continuation.finish() }
        defer { pending.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        let restored = try #require(app.workspace)
        try #require(restored.user.is_admin == wasAdmin)
        release.continuation.finish()
        await pending.value
        #expect(app.workspace === restored)
        #expect(restored.user == latest)
        #expect(try await cache.profile(connection: connection) == latest)
        #expect(app.error == nil)
    }

    @Test("本人情報の保存失敗でも検証済みの接続を使い、次の接続で保存を再試行する")
    func profileSaveFailureKeepsValidatedSession() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let fails = Mutex(false), requests = Mutex(0)
        let cache = try await SyncCache.open(inMemory: true, beforeSave: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let connection = SyncCache.connectionKey(harness.api.settings)
        _ = try await cache.activate(connection: connection, user: syncUser)
        var latest = syncUser
        latest.is_admin = true
        latest.display_name = "最新の本人情報"
        let response = try JSONEncoder().encode(latest)
        harness.stub { request in
            #expect(request.url?.path == "/api/users/me")
            if requests.withLock({ $0 += 1; return $0 }) == 1 { fails.withLock { $0 = true } }
            return (200, [:], response)
        }
        let app = AppSession(settings: harness.api.settings, transport: harness.api.transport,
            notificationStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys(), syncCache: cache, voipStore: PreviewPushReceiptStore())
        defer { app.logout() }
        await app.resume()
        let workspace = try #require(app.workspace)
        #expect(workspace.user == latest)
        #expect(app.error?.contains("保存できませんでした") == true)
        #expect(try await app.prepareForShortcut() === workspace)
        #expect(app.error != nil)
        fails.withLock { $0 = false }
        await app.resume()
        #expect(app.error == nil)
        #expect(try await cache.profile(connection: connection) == latest)
        let completedRequests = requests.withLock { $0 }
        #expect(try await app.prepareForShortcut() === workspace)
        #expect(requests.withLock { $0 } == completedRequests)
    }

    @Test("保存したログイン状態があれば通信なしでワークスペースを復元する")
    func offlineLaunch() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let preferenceKey = "zoo.push.enabled." + PushReceipt.digest(harness.api.settings.storageKey + "|" + syncUser.id)
        UserDefaults.standard.set(true, forKey: preferenceKey)
        defer { UserDefaults.standard.removeObject(forKey: preferenceKey) }
        try await sync.store([Channel(id: syncChannel, channel_name: "保存済み", is_private: false, is_dm: false)], key: "workspace:channels")
        try await sync.store([syncUser], key: "workspace:people")
        try await sync.store([ChannelReadState(channel_id: syncChannel, has_unread: false, last_read_at: nil)], key: "rest:api/channel-read-states")
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        let app = AppSession(settings: harness.api.settings, transport: harness.api.transport,
                             notificationStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys(), syncCache: sync.cache, voipStore: PreviewPushReceiptStore())
        await app.resume()
        #expect(app.userID == syncUser.id)
        let workspace = try #require(app.workspace)
        #expect(try await workspace.client.cachedWorkspace()?.channels.first?.channel_name == "保存済み")
        #expect(app.notifications.receive(.test(userID: syncUser.id, id: UUID())))
        let verified = Mutex(false)
        harness.stub { request in
            #expect(request.url?.path == "/api/users/me")
            verified.withLock { $0 = true }
            return (200, [:], try JSONEncoder().encode(syncUser))
        }
        await app.resume()
        #expect(verified.withLock { $0 })
        #expect(app.workspace === workspace)
        app.logout()
        #expect(app.workspace == nil)
    }
}
