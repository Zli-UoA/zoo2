import Foundation

@MainActor
final class SyncSession {
    let cache: SyncCache
    let lease: SyncLease
    private(set) var active = true
    private var revokedChannels: Set<String> = []
    private var uncachedChannels: Set<String> = []
    private var channelVersions: [String: UUID] = [:]
    private var revocationVersion = UUID()
    private var channelRevocation: Task<Void, Error>?
    private(set) var knownChannels: [String: Channel] = [:]
    private(set) var knownChannelsRevision = UUID()
    var onChannelRevoked: (@MainActor (String, Channel?) -> Void)?
    var onChannelAuthorized: (@MainActor (String) -> Void)?

    init(cache: SyncCache, lease: SyncLease) { self.cache = cache; self.lease = lease }

    func check(channelID: String? = nil, channelVersion expected: UUID? = nil) throws {
        try Task.checkCancellation()
        guard active, channelID.map({ !revokedChannels.contains($0) }) ?? true else { throw CancellationError() }
        if let channelID, let expected, channelVersion(channelID) != expected { throw CancellationError() }
    }

    func load<T: Decodable & Sendable>(_ key: String, as type: T.Type) async throws -> T? {
        try check()
        let version = revocationVersion
        // 永続削除前の読み取りと、デコード中の失効を会話単位の値でも遮断する。
        _ = await channelRevocation?.result
        try check()
        try checkRevocationVersion(version)
        let cached = try await cache.loadVersioned(key, lease: lease)
        let data = cached?.payload
        try check()
        do {
            let value = try await data.asyncDecode(type)
            try check()
            try checkRevocationVersion(version)
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

    func rememberChannels(_ channels: [Channel]) {
        for channel in channels { knownChannels[channel.id] = channel.deleted_at == nil ? channel : nil }
        knownChannelsRevision = UUID()
    }

    func checkKnownChannelsRevision(_ expected: UUID) throws {
        guard knownChannelsRevision == expected else { throw SyncInvalidation.resources }
    }

    func revocationVersion(for path: String) -> UUID? {
        let key = path.hasPrefix("api/") ? "snapshot:" + path : path
        return SyncCache.revocationResources.contains(key) ? revocationVersion : nil
    }

    func checkRevocationVersion(_ expected: UUID?) throws {
        if let expected, expected != revocationVersion { throw SyncInvalidation.resources }
    }

    func authorizeChannel(_ id: String, version: UUID) async throws {
        try check()
        guard channelVersion(id) == version else { throw CancellationError() }
        guard revokedChannels.contains(id) else { return }
        _ = await channelRevocation?.result
        try check()
        guard channelVersion(id) == version else { throw CancellationError() }
        if !uncachedChannels.contains(id) { try await cache.authorizeChannel(id, version: version, lease: lease) }
        try check()
        guard channelVersion(id) == version else { throw CancellationError() }
        revokedChannels.remove(id)
        onChannelAuthorized?(id)
    }

    func store<T: Encodable & Sendable>(_ value: T, key: String, channelID: String? = nil,
                                        channelVersion expectedVersion: UUID? = nil, revocationVersion: UUID? = nil) async throws {
        try check(channelID: channelID, channelVersion: expectedVersion)
        try checkRevocationVersion(revocationVersion)
        if let channelID, uncachedChannels.contains(channelID) { return }
        let data = try await encodeSyncValue(value)
        try check(channelID: channelID, channelVersion: expectedVersion)
        try checkRevocationVersion(revocationVersion)
        if let channelID, uncachedChannels.contains(channelID) { return }
        try await cache.store(data, resource: key, channelID: channelID, channelVersion: expectedVersion,
                              revocationVersion: revocationVersion, lease: lease)
        try check(channelID: channelID, channelVersion: expectedVersion)
        try checkRevocationVersion(revocationVersion)
    }

    /// 暗号化への切替では平文キャッシュだけを無効にし、閲覧権限と画面の世代は維持する。
    @discardableResult
    func excludeFromCache(_ id: String, version: UUID) async throws -> UUID {
        try check()
        guard channelVersion(id) == version else { throw CancellationError() }
        guard uncachedChannels.insert(id).inserted else { return revocationVersion }
        revocationVersion = UUID()
        let previous = channelRevocation
        let revocationVersion = revocationVersion
        let removal = Task { [cache, lease] in
            _ = await previous?.result
            try await cache.revokeChannel(id, version: version, revocationVersion: revocationVersion, lease: lease)
        }
        channelRevocation = removal
        // 削除失敗時も cache 側の読取・書込禁止を保ち、暗号文の表示は続ける。
        _ = await removal.result
        try check()
        guard channelVersion(id) == version else { throw CancellationError() }
        return revocationVersion
    }

    @discardableResult
    func revokeChannel(_ id: String, retaining channel: Channel? = nil) async throws -> UUID {
        try check()
        revokedChannels.insert(id)
        let version = UUID()
        channelVersions[id] = version
        revocationVersion = UUID()
        knownChannels[id] = channel
        onChannelRevoked?(id, channel)
        let previous = channelRevocation
        let revocationVersion = revocationVersion
        let revocation = Task { [cache, lease] in
            _ = await previous?.result
            try await cache.revokeChannel(id, version: version, revocationVersion: revocationVersion, lease: lease)
        }
        channelRevocation = revocation
        do { try await revocation.value }
        catch {
            // キャッシュの削除失敗で403/404や削除済みの会話情報を隠さず、画面でも失効を反映する。
            try check()
            if error is CancellationError || error is SyncInvalidation { throw error }
        }
        return revocationVersion
    }

    func storeRecovery<T: Encodable & Sendable>(_ value: T, key: String, channelID: String? = nil,
                                               channelVersion: UUID? = nil, revocationVersion: UUID? = nil) async throws {
        do { try await store(value, key: key, channelID: channelID, channelVersion: channelVersion,
                             revocationVersion: revocationVersion) }
        catch {
            try check(channelID: channelID, channelVersion: channelVersion)
            try checkRevocationVersion(revocationVersion)
            if error is CancellationError || error is SyncInvalidation { throw error }
        }
    }

    func close() -> Task<Void, Error> {
        active = false
        onChannelRevoked = nil
        onChannelAuthorized = nil
        knownChannels = [:]
        return Task { try await cache.revoke(lease) }
    }
}
