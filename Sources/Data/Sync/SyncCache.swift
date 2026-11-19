import CryptoKit
import Foundation
import SwiftData

@Model
final class CachedSyncRecord {
    @Attribute(.unique) var id: String
    var account: String
    var resource: String
    var channelID: String?
    var payload: Data

    init(account: String, resource: String, channelID: String?, payload: Data) {
        id = account + "|" + resource
        self.account = account
        self.resource = resource
        self.channelID = channelID
        self.payload = payload
    }
}

@Model
final class CachedSyncAccount {
    @Attribute(.unique) var connection: String
    var account: String
    var profile: Data

    init(connection: String, account: String, profile: Data) {
        self.connection = connection
        self.account = account
        self.profile = profile
    }
}

struct SyncLease: Sendable {
    let account: String
    let generation: UUID
}

enum SyncInvalidation: Error { case notifications }

// 行と同期位置を同じレコードに保存し、強制終了で片方だけ進むことを防ぐ。
@ModelActor
actor SyncCache {
    private var leases: [String: UUID] = [:]
    private var channelVersions: [String: UUID] = [:]
    private var blockedChannels: Set<String> = []
    private var notificationVersions: [String: UUID] = [:]
    private var attachmentFiles: AttachmentDiskCache?

    @concurrent static func open(inMemory: Bool = false, url: URL? = nil, attachmentDirectory: URL? = nil) async throws -> SyncCache {
        let schema = Schema([CachedSyncRecord.self, CachedSyncAccount.self])
        let configuration: ModelConfiguration
        if let url {
            configuration = ModelConfiguration(schema: schema, url: url, cloudKitDatabase: .none)
        } else {
            configuration = ModelConfiguration("ElectricCache", schema: schema,
                                               isStoredInMemoryOnly: inMemory, cloudKitDatabase: .none)
        }
        let cache = try SyncCache(modelContainer: ModelContainer(for: schema, configurations: configuration))
        let directory = attachmentDirectory ?? url.map { $0.appendingPathExtension("attachments") }
            ?? (inMemory ? nil : URL.cachesDirectory.appendingPathComponent("ZooAttachments-v1"))
        await cache.configureAttachments(directory)
        return cache
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
        return try modelContext.fetch(FetchDescriptor<CachedSyncRecord>(predicate: #Predicate { $0.id == id })).first?.payload
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
        let id = lease.account + "|" + resource
        if let row = try modelContext.fetch(FetchDescriptor<CachedSyncRecord>(predicate: #Predicate { $0.id == id })).first {
            row.payload = payload
        } else {
            modelContext.insert(CachedSyncRecord(account: lease.account, resource: resource, channelID: channelID, payload: payload))
        }
        try save()
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
        try? attachmentFiles?.remove(account: account, channelID: channelID)
        try modelContext.delete(model: CachedSyncRecord.self, where: #Predicate { $0.account == account && $0.channelID == channelID })
        // 通知の本文にも会話の情報が含まれるため、通知の控えも再取得する。
        let resources = ["shape:api/shapes/notifications", "snapshot:api/shapes/notifications"]
        for resource in resources { try remove(resource, lease: lease) }
        try save()
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
        do { try modelContext.save() }
        catch { modelContext.rollback(); throw error }
    }
}

@MainActor
final class SyncSession {
    let cache: SyncCache
    let lease: SyncLease
    private(set) var active = true
    private var revokedChannels: Set<String> = []
    private var channelVersions: [String: UUID] = [:]
    private var notificationVersion = UUID()
    var onChannelRevoked: (@MainActor (String) -> Void)?

    init(cache: SyncCache, lease: SyncLease) { self.cache = cache; self.lease = lease }

    func check(channelID: String? = nil, channelVersion expected: UUID? = nil) throws {
        try Task.checkCancellation()
        guard active, channelID.map({ !revokedChannels.contains($0) }) ?? true else { throw CancellationError() }
        if let channelID, let expected, channelVersion(channelID) != expected { throw CancellationError() }
    }

    func load<T: Decodable & Sendable>(_ key: String, as type: T.Type) async throws -> T? {
        try check()
        let data = try await cache.load(key, lease: lease)
        try check()
        do {
            let value = try await data.asyncDecode(type)
            try check()
            return value
        } catch is DecodingError {
            // キャッシュはサーバーから再構築できるため、旧形式の値で同期を止めない。
            try await cache.remove(key, lease: lease)
            return nil
        }
    }

    func channelVersion(_ id: String) -> UUID {
        if let version = channelVersions[id] { return version }
        let version = UUID()
        channelVersions[id] = version
        return version
    }

    func isRevoked(_ id: String) -> Bool { revokedChannels.contains(id) }

    func notificationVersion(for path: String) -> UUID? {
        path == "api/shapes/notifications" ? notificationVersion : nil
    }

    func checkNotificationVersion(_ expected: UUID?) throws {
        if let expected, expected != notificationVersion { throw SyncInvalidation.notifications }
    }

    func authorizeChannel(_ id: String, version: UUID) async throws {
        try check()
        guard channelVersion(id) == version else { throw CancellationError() }
        guard revokedChannels.contains(id) else { return }
        try await cache.authorizeChannel(id, version: version, lease: lease)
        try check()
        guard channelVersion(id) == version else { throw CancellationError() }
        revokedChannels.remove(id)
    }

    func store<T: Encodable & Sendable>(_ value: T, key: String, channelID: String? = nil,
                                        channelVersion expectedVersion: UUID? = nil, notificationVersion: UUID? = nil) async throws {
        try check(channelID: channelID, channelVersion: expectedVersion)
        try checkNotificationVersion(notificationVersion)
        let data = try await encodeSyncValue(value)
        try check(channelID: channelID, channelVersion: expectedVersion)
        try checkNotificationVersion(notificationVersion)
        try await cache.store(data, resource: key, channelID: channelID, channelVersion: expectedVersion,
                              notificationVersion: notificationVersion, lease: lease)
        try check(channelID: channelID, channelVersion: expectedVersion)
        try checkNotificationVersion(notificationVersion)
    }

    func revokeChannel(_ id: String) async throws {
        revokedChannels.insert(id)
        let version = UUID()
        channelVersions[id] = version
        notificationVersion = UUID()
        onChannelRevoked?(id)
        try await cache.revokeChannel(id, version: version, notificationVersion: notificationVersion, lease: lease)
    }

    func close() -> Task<Void, Error> {
        active = false
        return Task { try await cache.revoke(lease) }
    }
}

@concurrent func encodeSyncValue<T: Encodable & Sendable>(_ value: T) async throws -> Data {
    try JSONEncoder().encode(value)
}

extension Optional where Wrapped == Data {
    @concurrent func asyncDecode<T: Decodable & Sendable>(_ type: T.Type) async throws -> T? {
        try map { try JSONDecoder().decode(type, from: $0) }
    }
}

func isOfflineFailure(_ error: Error) -> Bool {
    guard let error = error as? URLError else { return false }
    return [.notConnectedToInternet, .networkConnectionLost, .timedOut, .cannotFindHost, .cannotConnectToHost,
            .dnsLookupFailed, .internationalRoamingOff, .dataNotAllowed].contains(error.code)
}
