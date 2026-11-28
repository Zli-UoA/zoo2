import CryptoKit
import Foundation
import SwiftData

struct SyncCacheValue: Sendable {
    let payload: Data
    let revision: UUID
}

// 行と同期位置を同じレコードに保存し、強制終了で片方だけ進むことを防ぐ。
@ModelActor
actor SyncCache {
    private var leases: [String: UUID] = [:]
    private var recordVersions: [String: UUID] = [:]
    private var channelVersions: [String: UUID] = [:]
    private var blockedChannels: Set<String> = []
    private var revocationVersions: [String: UUID] = [:]
    private var memoryPermissions: Set<String> = []
    private var revokedPermissions: Set<String> = []
    private var permissionsDirectory: URL?
    private var revocations: (any CacheRevocationStorage)?
    static let revocationResources = ["shape:api/shapes/notifications", "snapshot:api/shapes/notifications",
                                      "shape:api/shapes/channels", "snapshot:api/shapes/channels", "workspace:channels",
                                      "shape:api/shapes/user_pinned_channels", "snapshot:api/shapes/user_pinned_channels",
                                      "rest:api/channel-read-states", "rest:api/channel-pins"]
    private var beforeSave: (@Sendable () throws -> Void)?
    private var beforeDelete: (@Sendable () throws -> Void)?
    private var attachmentFiles: AttachmentDiskCache?

    @concurrent static func open(inMemory: Bool = false, url: URL? = nil, attachmentDirectory: URL? = nil,
                                beforeSave: (@Sendable () throws -> Void)? = nil,
                                beforeDelete: (@Sendable () throws -> Void)? = nil,
                                revocations: (any CacheRevocationStorage)? = nil) async throws -> SyncCache {
        let schema = Schema([CachedSyncRecord.self, CachedSyncAccount.self])
        let configuration: ModelConfiguration
        if let url {
            configuration = ModelConfiguration(schema: schema, url: url, cloudKitDatabase: .none)
        } else {
            configuration = ModelConfiguration("ElectricCache", schema: schema,
                                               isStoredInMemoryOnly: inMemory, cloudKitDatabase: .none)
        }
        let cache = try SyncCache(modelContainer: ModelContainer(for: schema, configurations: configuration))
        // 本番のコンテナ絶対パスはアプリ更新で変わるため、Keychain では固定の論理名を使う。
        // 明示 URL の独立ストアは、テストなどで同じ利用者の記録が混ざらないよう分ける。
        let namespace = url.map { $0.appendingPathExtension("permissions").path } ?? "ElectricCache-v1"
        await cache.configurePermissions(inMemory && url == nil ? nil : configuration.url.appendingPathExtension("permissions"),
                                         beforeSave: beforeSave, beforeDelete: beforeDelete,
                                         revocations: revocations ?? (inMemory && url == nil ? nil : KeychainCacheRevocations(namespace: namespace)))
        let directory = attachmentDirectory ?? url.map { $0.appendingPathExtension("attachments") }
            ?? (inMemory ? nil : URL.cachesDirectory.appendingPathComponent("ZooAttachments-v1"))
        await cache.configureAttachments(directory)
        return cache
    }

    private func configurePermissions(_ directory: URL?, beforeSave: (@Sendable () throws -> Void)?,
                                      beforeDelete: (@Sendable () throws -> Void)?, revocations: (any CacheRevocationStorage)?) {
        permissionsDirectory = directory
        self.revocations = revocations
        self.beforeSave = beforeSave
        self.beforeDelete = beforeDelete
    }

    private func permissionDirectory(account: String) -> URL? {
        let accountHash = SHA256.hash(data: Data(account.utf8)).map { String(format: "%02x", $0) }.joined()
        return permissionsDirectory?.appendingPathComponent(accountHash)
    }

    private func permissionURL(_ scope: String, account: String) -> URL? {
        let scopeHash = SHA256.hash(data: Data(scope.utf8)).map { String(format: "%02x", $0) }.joined()
        return permissionDirectory(account: account)?.appendingPathComponent(scopeHash)
    }

    private func hasPermission(_ scope: String, account: String) throws -> Bool {
        let key = account + "|" + scope
        guard !revokedPermissions.contains(key) else { return false }
        // 照合不能と失効を区別し、一時障害だけで保存済みの行を削除しない。
        if let revocations, try revocations.contains(key) { return false }
        guard let url = permissionURL(scope, account: account) else { return memoryPermissions.contains(key) }
        return (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
    }

    private func grantPermission(_ scope: String, account: String) throws {
        guard try !hasPermission(scope, account: account) else { return }
        if let url = permissionURL(scope, account: account) {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data().write(to: url, options: .atomic)
        }
        let key = account + "|" + scope
        if let revocations, try revocations.contains(key) { try revocations.allow(key) }
        memoryPermissions.insert(key)
        revokedPermissions.remove(key)
    }

    private func removePermission(_ scope: String, account: String) throws {
        let key = account + "|" + scope
        revokedPermissions.insert(key)
        memoryPermissions.remove(key)
        var failure: (any Error)?
        do { try revocations?.revoke(key) } catch { failure = error }
        if let url = permissionURL(scope, account: account) {
            do { try FileManager.default.removeItem(at: url) }
            catch CocoaError.fileNoSuchFile { }
            catch { if failure == nil { failure = error } }
        }
        if let failure { throw failure }
    }

    private func permissionScopes(resource: String, channelID: String?) -> [String] {
        var scopes = channelID.map { ["channel:" + $0] } ?? []
        if Self.revocationResources.contains(resource) { scopes.append("resource:" + resource) }
        return scopes
    }

    private func configureAttachments(_ directory: URL?) {
        attachmentFiles = directory.map { AttachmentDiskCache(directory: $0) }
        let prefix = "attachment-files:"
        if let records = try? modelContext.fetch(FetchDescriptor<CachedSyncRecord>(predicate: #Predicate { $0.resource.starts(with: prefix) })) {
            attachmentFiles?.removeOrphans(keeping: Set(records.compactMap { String(data: $0.payload, encoding: .utf8) }))
        }
    }

    static func connectionKey(_ settings: ConnectionSettings) -> String {
        SHA256.hash(data: Data(settings.storageKey.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    func profile(connection: String) throws -> Person? {
        let rows = try modelContext.fetch(FetchDescriptor<CachedSyncAccount>(predicate: #Predicate { $0.connection == connection }))
        guard let row = rows.first else { return nil }
        if let revocations, try revocations.contains(row.account + "|account") { return nil }
        return try JSONDecoder().decode(Person.self, from: row.profile)
    }

    func updateProfile(_ user: Person, lease: SyncLease) throws {
        try check(lease)
        let account = lease.account
        guard let row = try modelContext.fetch(FetchDescriptor<CachedSyncAccount>(predicate: #Predicate { $0.account == account })).first,
              account == row.connection + ":" + user.id else { throw CancellationError() }
        row.profile = try JSONEncoder().encode(user)
        try save()
    }

    func activate(connection: String, user: Person) throws -> SyncLease {
        // 取り消された復元が、後続ログインのleaseや保存済みアカウントを変更しない。
        try Task.checkCancellation()
        let account = connection + ":" + user.id
        let accountRevoked = try revocations?.contains(account + "|account") ?? false
        if accountRevoked {
            // ログアウト時の DB 削除が失敗した場合は、再ログインでも古い行を引き継がない。
            try deleteRecords(account: account)
        }
        let data = try JSONEncoder().encode(user)
        let rows = try modelContext.fetch(FetchDescriptor<CachedSyncAccount>(predicate: #Predicate { $0.connection == connection }))
        if let row = rows.first {
            if row.account != account {
                leases[row.account] = nil
                try deleteRecords(account: row.account)
            }
            row.account = account
            row.profile = data
        } else {
            modelContext.insert(CachedSyncAccount(connection: connection, account: account, profile: data))
        }
        try save()
        if accountRevoked { try revocations?.allow(account + "|account") }
        let lease = SyncLease(account: account, generation: UUID())
        leases[account] = lease.generation
        channelVersions = channelVersions.filter { !$0.key.hasPrefix(account + ":") }
        blockedChannels = blockedChannels.filter { !$0.hasPrefix(account + ":") }
        revocationVersions[account] = nil
        return lease
    }

    func load(_ resource: String, lease: SyncLease) throws -> Data? {
        try check(lease)
        if let revocations, (try? revocations.contains(lease.account + "|account")) != false { return nil }
        let id = lease.account + "|" + resource
        guard let row = try modelContext.fetch(FetchDescriptor<CachedSyncRecord>(predicate: #Predicate { $0.id == id })).first else { return nil }
        if let channelID = row.channelID, blockedChannels.contains(lease.account + ":" + channelID) { return nil }
        guard (try? permissionScopes(resource: resource, channelID: row.channelID)
            .allSatisfy({ try hasPermission($0, account: lease.account) })) == true else { return nil }
        return row.payload
    }

    func loadVersioned(_ resource: String, lease: SyncLease) throws -> SyncCacheValue? {
        guard let data = try load(resource, lease: lease) else { return nil }
        let id = lease.account + "|" + resource
        let revision = recordVersions[id] ?? UUID()
        recordVersions[id] = revision
        return SyncCacheValue(payload: data, revision: revision)
    }

    func store(_ payload: Data, resource: String, channelID: String? = nil, channelVersion: UUID? = nil,
               revocationVersion: UUID? = nil, lease: SyncLease) throws {
        try check(lease)
        if let revocationVersion {
            if let current = revocationVersions[lease.account], current != revocationVersion { throw SyncInvalidation.resources }
            revocationVersions[lease.account] = revocationVersion
        }
        if let channelID {
            let key = lease.account + ":" + channelID
            guard !blockedChannels.contains(key) else { throw CancellationError() }
            if let channelVersion {
                if let current = channelVersions[key], current != channelVersion { throw CancellationError() }
                channelVersions[key] = channelVersion
            }
        }
        let account = lease.account
        let scopes = permissionScopes(resource: resource, channelID: channelID)
        let permissions = try Dictionary(uniqueKeysWithValues: scopes.map { ($0, try hasPermission($0, account: account)) })
        if let channelID, permissions["channel:" + channelID] != true {
            // 新しい本文だけで許可を戻し、失効前のリアクションや添付の保存先を復活させない。
            try modelContext.delete(model: CachedSyncRecord.self, where: #Predicate { $0.account == account && $0.channelID == channelID })
        }
        let id = account + "|" + resource
        if let row = try modelContext.fetch(FetchDescriptor<CachedSyncRecord>(predicate: #Predicate { $0.id == id })).first {
            // ピン操作後に遅れたGETが届いても、次回起動用の一覧を古い版へ戻さない。
            if resource == "rest:api/channel-pins",
               let previous = try? JSONDecoder().decode(ChannelPins.self, from: row.payload),
               let incoming = try? JSONDecoder().decode(ChannelPins.self, from: payload),
               previous.version > incoming.version { return }
            row.payload = payload
        } else {
            modelContext.insert(CachedSyncRecord(account: lease.account, resource: resource, channelID: channelID, payload: payload))
        }
        try save()
        if recordVersions[id] != nil { recordVersions[id] = UUID() }
        for scope in permissionScopes(resource: resource, channelID: channelID) {
            try grantPermission(scope, account: account)
        }
    }

    func remove(_ resource: String, matching revision: UUID? = nil, lease: SyncLease) throws {
        guard leases[lease.account] == lease.generation else { throw CancellationError() }
        let id = lease.account + "|" + resource
        if let revision, recordVersions[id] != revision { return }
        try modelContext.delete(model: CachedSyncRecord.self, where: #Predicate { $0.id == id })
        try save()
        recordVersions[id] = nil
    }

    func loadAttachment(_ attachment: MessageAttachment, channelID: String, version: UUID,
                        reload: Bool, lease: SyncLease) throws -> Data? {
        try check(lease)
        try checkChannel(channelID, version: version, lease: lease)
        guard let attachmentFiles else { return nil }
        guard let namespace = try? attachmentNamespace(channelID: channelID, version: version, lease: lease) else { return nil }
        if reload {
            try? attachmentFiles.remove(account: lease.account, channelID: channelID, namespace: namespace, attachmentID: attachment.id)
            return nil
        }
        // キャッシュがOSに消された場合や読み出せない場合は、通常の取得へ戻す。
        return try? attachmentFiles.load(attachment, account: lease.account, channelID: channelID, namespace: namespace)
    }

    func storeAttachment(_ data: Data, attachment: MessageAttachment, channelID: String,
                         version: UUID, lease: SyncLease) throws {
        try check(lease)
        try checkChannel(channelID, version: version, lease: lease)
        guard let attachmentFiles else { return }
        guard let namespace = try? attachmentNamespace(channelID: channelID, version: version, lease: lease) else { return }
        // 空き容量不足でも、通信で取得した添付の表示は継続する。
        try? attachmentFiles.store(data, attachment: attachment, account: lease.account, channelID: channelID, namespace: namespace)
    }

    private func attachmentNamespace(channelID: String, version: UUID, lease: SyncLease) throws -> String {
        let resource = "attachment-files:" + channelID
        if let data = try load(resource, lease: lease), let value = String(data: data, encoding: .utf8), UUID(uuidString: value) != nil {
            return value
        }
        // 削除不能な古いファイルが残っても、失効後は別の保存先を使う。
        let value = UUID().uuidString
        try store(Data(value.utf8), resource: resource, channelID: channelID, channelVersion: version, lease: lease)
        return value
    }

    private func checkChannel(_ id: String, version: UUID, lease: SyncLease) throws {
        let key = lease.account + ":" + id
        guard !blockedChannels.contains(key), channelVersions[key].map({ $0 == version }) ?? true else { throw CancellationError() }
        channelVersions[key] = version
    }

    func revokeChannel(_ channelID: String, version: UUID = UUID(), revocationVersion: UUID = UUID(), lease: SyncLease) throws {
        guard leases[lease.account] == lease.generation else { throw CancellationError() }
        let account = lease.account
        let key = account + ":" + channelID
        channelVersions[key] = version
        blockedChannels.insert(key)
        revocationVersions[account] = revocationVersion
        let ids = Self.revocationResources.map { account + "|" + $0 }
        var failure: (any Error)?
        // 保存時に作った許可を先に消し、容量不足で失効記録を書けなくても再起動後の読出しを拒否する。
        for scope in ["channel:" + channelID] + Self.revocationResources.map({ "resource:" + $0 }) {
            do { try removePermission(scope, account: account) }
            catch { if failure == nil { failure = error } }
        }
        try? attachmentFiles?.remove(account: account, channelID: channelID)
        do {
            try beforeDelete?()
            try modelContext.delete(model: CachedSyncRecord.self, where: #Predicate { $0.account == account && $0.channelID == channelID })
            for id in ids { try modelContext.delete(model: CachedSyncRecord.self, where: #Predicate { $0.id == id }) }
            try save()
        } catch {
            modelContext.rollback()
            if failure == nil { failure = error }
        }
        if let failure { throw failure }
    }

    func authorizeChannel(_ channelID: String, version: UUID, lease: SyncLease) throws {
        try check(lease)
        let key = lease.account + ":" + channelID
        guard channelVersions[key] == version else { throw CancellationError() }
        blockedChannels.remove(key)
    }

    func revoke(_ lease: SyncLease) throws {
        guard leases[lease.account] == lease.generation else { return }
        leases[lease.account] = nil
        try deleteRecords(account: lease.account)
        let account = lease.account
        try modelContext.delete(model: CachedSyncAccount.self, where: #Predicate { $0.account == account })
        try save()
    }

    func forget(connection: String) throws {
        let accounts = try modelContext.fetch(FetchDescriptor<CachedSyncAccount>(predicate: #Predicate { $0.connection == connection }))
        for account in accounts {
            leases[account.account] = nil
            try deleteRecords(account: account.account)
            modelContext.delete(account)
        }
        try save()
    }

    private func deleteRecords(account: String) throws {
        // アカウント全体の削除でも、許可ディレクトリと DB の同時失敗に備える。
        try? revocations?.revoke(account + "|account")
        try? attachmentFiles?.remove(account: account)
        if let directory = permissionDirectory(account: account) { try? FileManager.default.removeItem(at: directory) }
        memoryPermissions = memoryPermissions.filter { !$0.hasPrefix(account + "|") }
        revokedPermissions = revokedPermissions.filter { !$0.hasPrefix(account + "|") }
        recordVersions = recordVersions.filter { !$0.key.hasPrefix(account + "|") }
        try beforeDelete?()
        try modelContext.delete(model: CachedSyncRecord.self, where: #Predicate { $0.account == account })
        channelVersions = channelVersions.filter { !$0.key.hasPrefix(account + ":") }
        blockedChannels = blockedChannels.filter { !$0.hasPrefix(account + ":") }
        revocationVersions[account] = nil
    }

    private func check(_ lease: SyncLease) throws {
        try Task.checkCancellation()
        guard leases[lease.account] == lease.generation else { throw CancellationError() }
    }

    private func save() throws {
        do { try beforeSave?(); try modelContext.save() }
        catch { modelContext.rollback(); throw error }
    }
}
