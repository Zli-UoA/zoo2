import CryptoKit
import Foundation
import SwiftData

// 行と同期位置を同じレコードに保存し、強制終了で片方だけ進むことを防ぐ。
@ModelActor
actor SyncCache {
    private var leases: [String: UUID] = [:]
    private var channelVersions: [String: UUID] = [:]
    private var blockedChannels: Set<String> = []
    private var notificationVersions: [String: UUID] = [:]
    private var memoryPermissions: Set<String> = []
    private var revokedPermissions: Set<String> = []
    private var permissionsDirectory: URL?
    private static let notificationResources = ["shape:api/shapes/notifications", "snapshot:api/shapes/notifications"]
    private var beforeSave: (@Sendable () throws -> Void)?
    private var attachmentFiles: AttachmentDiskCache?

    @concurrent static func open(inMemory: Bool = false, url: URL? = nil, attachmentDirectory: URL? = nil,
                                beforeSave: (@Sendable () throws -> Void)? = nil) async throws -> SyncCache {
        let schema = Schema([CachedSyncRecord.self, CachedSyncAccount.self])
        let configuration: ModelConfiguration
        if let url {
            configuration = ModelConfiguration(schema: schema, url: url, cloudKitDatabase: .none)
        } else {
            configuration = ModelConfiguration("ElectricCache", schema: schema,
                                               isStoredInMemoryOnly: inMemory, cloudKitDatabase: .none)
        }
        let cache = try SyncCache(modelContainer: ModelContainer(for: schema, configurations: configuration))
        await cache.configurePermissions(inMemory && url == nil ? nil : configuration.url.appendingPathExtension("permissions"),
                                             beforeSave: beforeSave)
        let directory = attachmentDirectory ?? url.map { $0.appendingPathExtension("attachments") }
            ?? (inMemory ? nil : URL.cachesDirectory.appendingPathComponent("ZooAttachments-v1"))
        await cache.configureAttachments(directory)
        return cache
    }

    private func configurePermissions(_ directory: URL?, beforeSave: (@Sendable () throws -> Void)?) {
        permissionsDirectory = directory
        self.beforeSave = beforeSave
    }

    private func permissionURL(_ scope: String, account: String) -> URL? {
        let accountHash = SHA256.hash(data: Data(account.utf8)).map { String(format: "%02x", $0) }.joined()
        let scopeHash = SHA256.hash(data: Data(scope.utf8)).map { String(format: "%02x", $0) }.joined()
        return permissionsDirectory?.appendingPathComponent(accountHash).appendingPathComponent(scopeHash)
    }

    private func hasPermission(_ scope: String, account: String) -> Bool {
        let key = account + "|" + scope
        guard !revokedPermissions.contains(key) else { return false }
        guard let url = permissionURL(scope, account: account) else { return memoryPermissions.contains(key) }
        return (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
    }

    private func grantPermission(_ scope: String, account: String) throws {
        guard !hasPermission(scope, account: account) else { return }
        if let url = permissionURL(scope, account: account) {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data().write(to: url, options: .atomic)
        }
        let key = account + "|" + scope
        memoryPermissions.insert(key)
        revokedPermissions.remove(key)
    }

    private func removePermission(_ scope: String, account: String) throws {
        let key = account + "|" + scope
        revokedPermissions.insert(key)
        memoryPermissions.remove(key)
        guard let url = permissionURL(scope, account: account) else { return }
        do { try FileManager.default.removeItem(at: url) }
        catch CocoaError.fileNoSuchFile { }
    }

    private func permissionScopes(resource: String, channelID: String?) -> [String] {
        var scopes = channelID.map { ["channel:" + $0] } ?? []
        if Self.notificationResources.contains(resource) { scopes.append("resource:" + resource) }
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
        return try rows.first.map { try JSONDecoder().decode(Person.self, from: $0.profile) }
    }

    func activate(connection: String, user: Person) throws -> SyncLease {
        let account = connection + ":" + user.id
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
        let lease = SyncLease(account: account, generation: UUID())
        leases[account] = lease.generation
        channelVersions = channelVersions.filter { !$0.key.hasPrefix(account + ":") }
        blockedChannels = blockedChannels.filter { !$0.hasPrefix(account + ":") }
        notificationVersions[account] = nil
        return lease
    }

    func load(_ resource: String, lease: SyncLease) throws -> Data? {
        try check(lease)
        let id = lease.account + "|" + resource
        guard let row = try modelContext.fetch(FetchDescriptor<CachedSyncRecord>(predicate: #Predicate { $0.id == id })).first else { return nil }
        if let channelID = row.channelID, blockedChannels.contains(lease.account + ":" + channelID) { return nil }
        guard permissionScopes(resource: resource, channelID: row.channelID).allSatisfy({ hasPermission($0, account: lease.account) }) else { return nil }
        return row.payload
    }

    func store(_ payload: Data, resource: String, channelID: String? = nil, channelVersion: UUID? = nil,
               notificationVersion: UUID? = nil, lease: SyncLease) throws {
        try check(lease)
        if let notificationVersion {
            if let current = notificationVersions[lease.account], current != notificationVersion { throw SyncInvalidation.notifications }
            notificationVersions[lease.account] = notificationVersion
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
        if let channelID, !hasPermission("channel:" + channelID, account: account) {
            // 新しい本文だけで許可を戻し、失効前のリアクションや添付の保存先を復活させない。
            try modelContext.delete(model: CachedSyncRecord.self, where: #Predicate { $0.account == account && $0.channelID == channelID })
        }
        let id = account + "|" + resource
        if let row = try modelContext.fetch(FetchDescriptor<CachedSyncRecord>(predicate: #Predicate { $0.id == id })).first {
            row.payload = payload
        } else {
            modelContext.insert(CachedSyncRecord(account: lease.account, resource: resource, channelID: channelID, payload: payload))
        }
        try save()
        for scope in permissionScopes(resource: resource, channelID: channelID) {
            try grantPermission(scope, account: account)
        }
    }

    func remove(_ resource: String, lease: SyncLease) throws {
        guard leases[lease.account] == lease.generation else { throw CancellationError() }
        let id = lease.account + "|" + resource
        try modelContext.delete(model: CachedSyncRecord.self, where: #Predicate { $0.id == id })
        try save()
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

    func revokeChannel(_ channelID: String, version: UUID = UUID(), notificationVersion: UUID = UUID(), lease: SyncLease) throws {
        guard leases[lease.account] == lease.generation else { throw CancellationError() }
        let account = lease.account
        let key = account + ":" + channelID
        channelVersions[key] = version
        blockedChannels.insert(key)
        notificationVersions[account] = notificationVersion
        let ids = Self.notificationResources.map { account + "|" + $0 }
        var failure: (any Error)?
        // 保存時に作った許可を先に消し、容量不足で失効記録を書けなくても再起動後の読出しを拒否する。
        for scope in ["channel:" + channelID] + Self.notificationResources.map({ "resource:" + $0 }) {
            do { try removePermission(scope, account: account) }
            catch { if failure == nil { failure = error } }
        }
        try? attachmentFiles?.remove(account: account, channelID: channelID)
        do {
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
        try? attachmentFiles?.remove(account: account)
        try modelContext.delete(model: CachedSyncRecord.self, where: #Predicate { $0.account == account })
        channelVersions = channelVersions.filter { !$0.key.hasPrefix(account + ":") }
        blockedChannels = blockedChannels.filter { !$0.hasPrefix(account + ":") }
        notificationVersions[account] = nil
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
