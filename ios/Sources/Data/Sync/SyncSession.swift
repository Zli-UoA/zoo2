import Foundation

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
        let cached = try await cache.loadVersioned(key, lease: lease)
        let data = cached?.payload
        try check()
        do {
            let value = try await data.asyncDecode(type)
            try check()
            return value
        } catch is DecodingError {
            // キャッシュはサーバーから再構築できるため、旧形式の値で同期を止めない。
            if let cached { try await cache.remove(key, matching: cached.revision, lease: lease) }
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
        do { try await cache.revokeChannel(id, version: version, notificationVersion: notificationVersion, lease: lease) }
        catch {
            // キャッシュの削除失敗で403/404や削除済みの会話情報を隠さず、画面でも失効を反映する。
            try check()
            if error is CancellationError || error is SyncInvalidation { throw error }
        }
    }

    func storeRecovery<T: Encodable & Sendable>(_ value: T, key: String) async throws {
        do { try await store(value, key: key) }
        catch {
            try check()
            if error is CancellationError || error is SyncInvalidation { throw error }
        }
    }

    func close() -> Task<Void, Error> {
        active = false
        return Task { try await cache.revoke(lease) }
    }
}
