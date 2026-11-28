import Foundation

@MainActor
struct NativeChatClient: ChatClient {
    let api: APIClient
    var sync: SyncSession? = nil
    var serverURL: String { api.settings.server }
    var shapes: ShapeStream { ShapeStream(api: api, sync: sync) }

    func channels() async throws -> [Channel] {
        guard let sync else { return try await api.get("api/channels") }
        var version = sync.revocationVersion(for: "workspace:channels")
        let knownRevision = sync.knownChannelsRevision
        var channels = try await shapes.snapshot("api/shapes/channels", as: Channel.self, restoreCached: false)
        let cached = try await sync.load("workspace:channels", as: [Channel].self) ?? []
        try sync.checkRevocationVersion(version)
        try sync.checkKnownChannelsRevision(knownRevision)
        var previous = Dictionary(cached.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
        previous.merge(sync.knownChannels) { _, current in current }
        var available = Dictionary(channels.filter { $0.deleted_at == nil }.map { ($0.id, $0) },
                                   uniquingKeysWith: { _, new in new })
        var verifiedAccess = false
        if channels.contains(where: { sync.isRevoked($0.id) }) || previous.values.contains(where: { channel in
            channel.deleted_at == nil && (available[channel.id] == nil
                || (channel.encrypted != true && available[channel.id]?.encrypted == true))
        }) {
            // ElectricはRESTより遅れるため、失効候補がある場合だけ正本一覧を一度照合する。
            channels = try await api.get("api/channels")
            verifiedAccess = true
            try sync.check()
            try sync.checkRevocationVersion(version)
            try sync.checkKnownChannelsRevision(knownRevision)
            available = Dictionary(channels.filter { $0.deleted_at == nil }.map { ($0.id, $0) },
                                   uniquingKeysWith: { _, new in new })
        }
        for channel in previous.values where channel.deleted_at == nil {
            let current = available[channel.id]
            if current == nil {
                try sync.checkRevocationVersion(version)
                version = try await sync.revokeChannel(channel.id)
                try sync.checkRevocationVersion(version)
                try sync.checkKnownChannelsRevision(knownRevision)
            }
        }
        for channel in channels where channel.deleted_at == nil && channel.encrypted == true {
            version = try await sync.excludeFromCache(channel.id, version: sync.channelVersion(channel.id))
            try sync.checkRevocationVersion(version)
            try sync.checkKnownChannelsRevision(knownRevision)
        }
        if verifiedAccess {
            for channel in channels where channel.deleted_at == nil {
                try sync.checkRevocationVersion(version)
                try sync.checkKnownChannelsRevision(knownRevision)
                try await sync.authorizeChannel(channel.id, version: sync.channelVersion(channel.id))
                try sync.checkRevocationVersion(version)
                try sync.checkKnownChannelsRevision(knownRevision)
            }
        }
        sync.rememberChannels(channels)
        let storedRevision = sync.knownChannelsRevision
        try await sync.storeRecovery(channels, key: "workspace:channels", revocationVersion: version)
        try sync.checkKnownChannelsRevision(storedRevision)
        return channels
    }
    func channel(id: String) async throws -> Channel {
        let version = sync?.channelVersion(id)
        do {
            let channel: Channel = try await api.get("api/channels/\(try segment(id))")
            if let sync, let version {
                try sync.check()
                guard sync.channelVersion(id) == version else { throw CancellationError() }
                if channel.deleted_at != nil {
                    try await sync.revokeChannel(id)
                } else if channel.encrypted == true {
                    try await sync.excludeFromCache(id, version: version)
                    try await sync.authorizeChannel(id, version: version)
                } else {
                    try await sync.authorizeChannel(id, version: version)
                }
                try sync.check()
                sync.rememberChannels([channel])
            }
            return channel
        } catch {
            if case ZooError.rejected(let status, _) = error, status == 403 || status == 404,
               let sync, sync.channelVersion(id) == version {
                try await sync.revokeChannel(id)
            }
            throw error
        }
    }
    func people() async throws -> [Person] {
        // REST のユーザー応答には avatar_url がないため、Web と同じ shape を使う。
        let people = try await shapes.snapshot("api/shapes/users", as: Person.self)
        try await sync?.storeRecovery(people, key: "workspace:people")
        return people
    }
    func person(id: String) async throws -> Person { try await api.get("api/users/\(try segment(id))") }
    func readStates(allowCached: Bool) async throws -> [ChannelReadState] {
        try await cachedGet("api/channel-read-states", allowCached: allowCached)
    }

    func cachedWorkspace() async throws -> CachedWorkspace? {
        guard let sync else { return nil }
        let version = sync.revocationVersion(for: "workspace:channels")
        let knownRevision = sync.knownChannelsRevision
        guard let channels = try await sync.load("workspace:channels", as: [Channel].self),
              let people = try await sync.load("workspace:people", as: [Person].self),
              let states = try await sync.load("rest:api/channel-read-states", as: [ChannelReadState].self) else { return nil }
        let pins = try await sync.load("rest:api/channel-pins", as: ChannelPins.self)
        try sync.checkRevocationVersion(version)
        try sync.checkKnownChannelsRevision(knownRevision)
        sync.rememberChannels(channels)
        return CachedWorkspace(channels: channels, people: people, readStates: states, pins: pins)
    }

    func cachedPeople() async throws -> [Person]? {
        try await sync?.load("workspace:people", as: [Person].self)
    }

    func cachedGet<T: Codable & Sendable>(_ path: String, query: [String: String] = [:],
                                         channelID: String? = nil, allowCached: Bool = true,
                                         allowOfflineFallback: Bool = true) async throws -> T {
        // 旧形式の会話単位の行には権限タグがないため、新しい名前空間だけを読む。
        let key = (channelID == nil ? "rest:" : "channel-rest:") + path
            + query.sorted { $0.key < $1.key }.map { "|\($0.key)=\($0.value)" }.joined()
        let version = channelID.flatMap { sync?.channelVersion($0) }
        let revocationVersion = sync?.revocationVersion(for: key)
        try sync?.check(channelID: channelID, channelVersion: version)
        do {
            let data = try await api.data(path, query: query)
            guard let value = try await Optional(data).asyncDecode(T.self) else { throw ZooError.invalidResponse }
            try sync?.check(channelID: channelID, channelVersion: version)
            try sync?.checkRevocationVersion(revocationVersion)
            if allowCached {
                try await sync?.storeRecovery(value, key: key, channelID: channelID, channelVersion: version,
                                              revocationVersion: revocationVersion)
            }
            return value
        } catch {
            try sync?.check(channelID: channelID, channelVersion: version)
            try sync?.checkRevocationVersion(revocationVersion)
            if case ZooError.rejected(let status, _) = error, status == 403 || status == 404, let channelID {
                try await sync?.revokeChannel(channelID)
            }
            if allowCached, allowOfflineFallback, isOfflineFailure(error), let value = try await sync?.load(key, as: T.self) {
                try sync?.check(channelID: channelID, channelVersion: version)
                try sync?.checkRevocationVersion(revocationVersion)
                return value
            }
            throw error
        }
    }

    func channelPins(allowOfflineFallback: Bool) async throws -> ChannelPins {
        try await cachedGet("api/channel-pins", allowOfflineFallback: allowOfflineFallback)
    }
    func observeChannelPinChanges(receive: @MainActor () -> Void) async throws {
        var previous: [ChannelPin]?
        try await shapes.observe("api/shapes/user_pinned_channels", query: [:], as: ChannelPin.self) { rows in
            guard let rows else { previous = nil; return }
            let current = rows.sorted { $0.channel_id < $1.channel_id }
            guard current != previous else { return }
            previous = current
            receive()
        }
    }
    func setChannelPinned(id: String, pinned: Bool, expectedVersion: Int64) async throws -> ChannelPins {
        let id = try segment(id)
        let revocationVersion = sync?.revocationVersion(for: "rest:api/channel-pins")
        let result: ChannelPins
        if pinned {
            result = try await api.write("api/channel-pins", body: ["channel_id": .string(id), "expected_version": .number(Double(expectedVersion))])
        } else {
            result = try await api.write("api/channel-pins/\(id)", method: "DELETE", query: ["expected_version": String(expectedVersion)])
        }
        try await sync?.storeRecovery(result, key: "rest:api/channel-pins", revocationVersion: revocationVersion)
        return result
    }
    func members(channelID: String) async throws -> [ChannelMembership] {
        try await shapes.snapshot("api/shapes/channel_users", query: ["channel_id": try segment(channelID)], as: ChannelMembership.self)
    }
    func channelNotificationSettings(id: String) async throws -> ChannelNotificationSettings {
        try await api.get("api/channels/\(try segment(id))/notification-settings")
    }
    func setChannelNotificationSettings(id: String, level: ChannelNotificationLevel) async throws -> Envelope<ChannelNotificationSettings> {
        let data = try await api.data("api/channels/\(try segment(id))/notification-settings", method: "PATCH",
                                      body: ["level": .string(level.rawValue)])
        return try JSONDecoder().decode(Envelope<ChannelNotificationSettings>.self, from: data)
    }
    func observeChannelNotificationSettings(receive: @MainActor (ChannelNotificationSnapshot) -> Void) async throws {
        var transactionIDs: Set<UInt64> = []
        var freshSnapshot = false
        try await shapes.observe("api/shapes/channel_notification_settings", query: [:], as: ChannelNotificationSetting.self,
                                 restoreCached: false, transactions: { ids, fresh in
            transactionIDs = ids
            freshSnapshot = fresh
        }) { rows in
            receive(ChannelNotificationSnapshot(rows: rows, transactionIDs: transactionIDs, freshSnapshot: freshSnapshot))
        }
    }
    func readState(channelID: String, allowCached: Bool) async throws -> ChannelReadState {
        let id = try segment(channelID)
        if sync?.isRevoked(id) == true { _ = try await channel(id: id) }
        let path = "api/channels/\(id)/read-state"
        return try await cachedGet(path, channelID: id, allowCached: allowCached)
    }
    func attachmentURL(id: String) async throws -> URL {
        let result: Envelope<AttachmentDownload> = try await api.get("api/attachments/\(try segment(id))/url")
        guard let url = result.data.url else { throw ZooError.invalidResponse }
        return url
    }

    func attachmentData(from url: URL) async throws -> Data {
        guard DisplayEmoji.assetURL(url.absoluteString) != nil else { throw ZooError.invalidResponse }
        // 署名付きURLにはAPIの認証トークンを付けず、ストレージの401でログアウトさせない。
        let (data, response) = try await api.transport.send(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData))
        try Task.checkCancellation()
        guard (200..<300).contains(response.statusCode) else {
            throw ZooError.rejected(response.statusCode, "添付ファイルを取得できませんでした。")
        }
        return data
    }

    func attachmentData(_ attachment: MessageAttachment, channelID: String? = nil, reload: Bool = false) async throws -> Data {
        if let fileID = attachment.encryptedFileID {
            // 暗号化した添付：暗号文を取り、ハッシュを確かめて復号する。平文はキャッシュに書かない。
            let version = channelID.flatMap { sync?.channelVersion($0) }
            try sync?.check()
            if let channelID { try sync?.check(channelID: channelID, channelVersion: version) }
            let url = try await attachmentURL(id: attachment.id)
            try Task.checkCancellation()
            let ciphertext = try await attachmentData(from: url)
            try sync?.check()
            if let channelID { try sync?.check(channelID: channelID, channelVersion: version) }
            guard let data = EncryptedAttachmentFiles.shared.decrypt(ciphertext, id: fileID) else {
                throw ZooError.rejected(422, "この添付は復号できませんでした。")
            }
            return data
        }
        guard let sync, let channelID, attachment.isImage || attachment.isPDF else {
            let version = channelID.flatMap { sync?.channelVersion($0) }
            if let channelID { try sync?.check(channelID: channelID, channelVersion: version) }
            let url = try await attachmentURL(id: attachment.id)
            try Task.checkCancellation()
            let data = try await attachmentData(from: url)
            if let channelID { try sync?.check(channelID: channelID, channelVersion: version) }
            return data
        }
        _ = try segment(attachment.id)
        _ = try segment(channelID)
        let version = sync.channelVersion(channelID)
        try sync.check(channelID: channelID, channelVersion: version)
        let cached = try await sync.loadAttachment(attachment, channelID: channelID, version: version, reload: reload)
        try sync.check(channelID: channelID, channelVersion: version)
        if let cached { return cached }
        let url: URL
        do { url = try await attachmentURL(id: attachment.id) }
        catch {
            try sync.check(channelID: channelID, channelVersion: version)
            if case ZooError.rejected(let status, _) = error, status == 403 || status == 404 {
                _ = try await sync.loadAttachment(attachment, channelID: channelID, version: version, reload: true)
            }
            throw error
        }
        try sync.check(channelID: channelID, channelVersion: version)
        let data = try await attachmentData(from: url)
        try sync.check(channelID: channelID, channelVersion: version)
        try await sync.storeAttachment(data, attachment: attachment, channelID: channelID, version: version)
        try sync.check(channelID: channelID, channelVersion: version)
        return data
    }

    func attachmentFile(_ attachment: MessageAttachment, channelID: String?) async throws -> AttachmentFile {
        let version = channelID.flatMap { sync?.channelVersion($0) }
        if let channelID { try sync?.check(channelID: channelID, channelVersion: version) }
        let file: AttachmentFile
        if attachment.encryptedFileID != nil {
            file = try await AttachmentFile.write(attachmentData(attachment, channelID: channelID))
        } else if attachment.isImage || attachment.isPDF {
            file = try await AttachmentFile.write(attachmentData(attachment, channelID: channelID))
        } else {
            let url = try await attachmentURL(id: attachment.id)
            guard DisplayEmoji.assetURL(url.absoluteString) != nil else { throw ZooError.invalidResponse }
            let (downloaded, response) = try await api.transport.download(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData))
            guard (200..<300).contains(response.statusCode) else {
                throw ZooError.rejected(response.statusCode, "添付ファイルを取得できませんでした。")
            }
            file = downloaded
        }
        try Task.checkCancellation()
        if let channelID { try sync?.check(channelID: channelID, channelVersion: version) }
        return file
    }

    func observeMessages(channelID: String, receive: @MainActor ([Message]?) -> Void) async throws {
        _ = try segment(channelID)
        try await ShapeStream(api: api).observe("api/shapes/messages", query: ["channel_id": channelID],
                                               as: Message.self, receive: receive)
    }

    func observeMessageFeed(channelID: String, request: MessageFeedRequest, receive: @MainActor (MessageFeedSnapshot) -> Void) async throws {
        guard let sync else {
            return try await observeMessages(channelID: channelID) { receive(MessageFeedSnapshot(messages: $0)) }
        }
        if sync.isRevoked(channelID) { _ = try await channel(id: channelID) }
        try await MessageSync(api: api, sync: sync, channelID: segment(channelID)).observe(request, receive: receive)
    }

    func observeAttachments(channelID: String, receive: @MainActor ([MessageAttachment]?) -> Void) async throws {
        try await shapes.observe("api/shapes/attachments", query: ["channel_id": try segment(channelID)], as: MessageAttachment.self, receive: receive)
    }

    func observeReactions(channelID: String, receive: @MainActor ([MessageReaction]?) -> Void) async throws {
        try await shapes.observe("api/shapes/message_reactions", query: ["channel_id": try segment(channelID)],
                                 as: MessageReaction.self, restoreCached: false, receive: receive)
    }

    func addReaction(messageID: String, emoji: String) async throws -> MessageReaction {
        try await api.write("api/messages/\(try segment(messageID))/reactions", body: ["emoji": .string(emoji)])
    }

    func removeReaction(messageID: String, emoji: String) async throws {
        _ = try await api.data("api/messages/\(try segment(messageID))/reactions", method: "DELETE", query: ["emoji": emoji])
    }

    func emojis() async throws -> [WorkspaceEmoji] {
        let catalog: [WorkspaceEmoji] = try await api.get("api/emojis")
        return catalog.map { $0.resolved(relativeTo: serverURL) }
    }

    func emojiAccounts() async throws -> [EmojiAccount] {
        struct Directory: Decodable { let accounts: [EmojiAccount] }
        let directory: Directory = try await api.get("api/emoji-accounts")
        return directory.accounts.map { $0.resolved(relativeTo: serverURL) }
    }

    func upload(_ attachment: DraftAttachment, channelID: String) async throws -> String {
        try await upload(file: attachment.file.url, name: attachment.fileName, size: attachment.byteCount,
                         contentType: attachment.contentType, channelID: channelID)
    }

    /// 暗号化した添付の暗号文を上げる。ファイル名と Content-Type は、決まった値（`encrypted.bin`、`application/octet-stream`）。
    func uploadEncrypted(_ ciphertext: Data, channelID: String) async throws -> String {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try ciphertext.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        defer { try? FileManager.default.removeItem(at: url) }
        return try await upload(file: url, name: "encrypted.bin", size: ciphertext.count,
                                contentType: "application/octet-stream", channelID: channelID)
    }

    private func upload(file: URL, name: String, size: Int, contentType: String, channelID: String) async throws -> String {
        let result: AttachmentUpload = try await api.write("api/attachments/presign", body: [
            "channel_id": .string(try segment(channelID)), "file_name": .string(name),
            "file_size": .number(Double(size)), "content_type": .string(contentType)
        ])
        let id = try segment(result.attachment_id)
        guard let url = URL(string: result.upload_url), url.scheme == "https", url.host != nil,
              url.user == nil, url.password == nil, url.fragment == nil else { throw ZooError.invalidResponse }
        try Task.checkCancellation()
        var request = URLRequest(url: url)
        request.httpMethod = "PUT"
        request.setValue(contentType, forHTTPHeaderField: "Content-Type")
        try await api.transport.upload(request, from: file)
        return id
    }

    func send(id: String, channelID: String, content: String, parentID: String?, attachmentIDs: [String] = [], sensitiveAttachmentIDs: [String] = []) async throws -> Message {
        _ = try segment(id)
        _ = try segment(channelID)
        var body: [String: JSONValue] = ["id": .string(id), "channel_id": .string(channelID), "content": .string(content)]
        if let parentID { body["parent_id"] = .string(try segment(parentID)) }
        if !attachmentIDs.isEmpty { body["attachment_ids"] = .array(try attachmentIDs.map { .string(try segment($0)) }) }
        if !sensitiveAttachmentIDs.isEmpty {
            body["sensitive_attachment_ids"] = .array(try sensitiveAttachmentIDs.map { .string(try segment($0)) })
        }
        do {
            return try await api.write("api/messages", body: body)
        } catch ZooError.threadJoinRequired(let message) {
            // 参加行があるスレッドへ、未参加・退出済みの利用者は返信できない（サーバーが 403 を返す）。
            // 参加（アーカイブ済みは先に再Open）してから、同じ ID で一度だけ再送する。復旧できなければ元の 403 を返す。
            // 投稿権限がないチャンネルなど、参加不足以外の 403 は `threadJoinRequired` にならず、参加せずに失敗する
            // （判定は `code` を優先し、`code` のない応答だけ本文で見る。`HTTPTransport.check`）。
            do {
                guard let parentID, try await api.rejoinThread(messageID: parentID) else {
                    throw ZooError.rejected(403, message)
                }
            } catch {
                try Task.checkCancellation()
                if error is CancellationError { throw error }
                if case ZooError.expiredSession = error { throw error }
                throw ZooError.rejected(403, message)
            }
            do {
                return try await api.write("api/messages", body: body)
            } catch ZooError.threadJoinRequired(let message) {
                // 参加したのに、また参加が必要と言われた（直後に退出された）。ほかの 403 と同じ失敗にする。
                throw ZooError.rejected(403, message)
            }
        }
    }

    func edit(id: String, content: String) async throws -> Message {
        try await api.write("api/messages/\(try segment(id))", method: "PATCH", body: ["content": .string(content)])
    }

    func delete(id: String) async throws {
        _ = try await api.data("api/messages/\(try segment(id))", method: "DELETE")
    }

    func markRead(channelID: String, messageID: String) async throws {
        _ = try await api.data("api/channels/\(try segment(channelID))/read-state", method: "POST",
                               body: ["message_id": .string(try segment(messageID))])
    }

    func markChannelRead(channelID: String) async throws -> ChannelReadState {
        let id = try segment(channelID)
        if sync?.isRevoked(id) == true { _ = try await channel(id: id) }
        let version = sync?.channelVersion(id)
        try sync?.check(channelID: id, channelVersion: version)
        // 全履歴や端末のキャッシュではなく、サーバーの最新の親投稿を1件だけ取得する。
        let data = try await api.data("api/shapes/messages", query: [
            "channel_id": id, "offset": "-1", "log": "changes_only", "replica": "full",
            "subset__where": "parent_id IS NULL AND deleted_at IS NULL",
            "subset__order_by": "created_at DESC, id DESC", "subset__limit": "1"
        ])
        let snapshot = try JSONDecoder().decode(Envelope<[ShapeEvent]>.self, from: data)
        try Task.checkCancellation()
        try sync?.check(channelID: id, channelVersion: version)
        let path = "api/channels/\(id)/read-state"
        let state: ChannelReadState
        if let event = snapshot.data.first {
            guard snapshot.data.count == 1, event.headers.operation == "insert",
                  let row = event.value, row["channel_id"]?.text == id,
                  row["parent_id"]?.text == nil, row["deleted_at"]?.text == nil,
                  let messageID = row["id"]?.text else { throw ZooError.invalidResponse }
            state = try await api.write(path, body: ["message_id": .string(try segment(messageID))])
        } else {
            state = try await api.get(path)
        }
        try Task.checkCancellation()
        try sync?.check(channelID: id, channelVersion: version)
        guard state.channel_id == id else { throw ZooError.invalidResponse }
        return state
    }

    func createChannel(id: String, name: String, description: String, isPrivate: Bool, encrypted: Bool) async throws -> Channel {
        var body: [String: JSONValue] = ["id": .string(try segment(id)), "channel_name": .string(name),
                                         "description": .string(description), "is_private": .bool(isPrivate)]
        if encrypted { body["encrypted"] = .bool(true) }
        let channel: Channel = try await api.write("api/channels", body: body)
        // 暗号化を求めたのに、そうでない応答は、格下げ（サーバーが平文のチャンネルにした）。受け入れない。
        if encrypted, channel.encrypted != true { throw MessageEncryptionError.downgraded }
        return channel
    }

    func directMessage(userIDs: [String], encrypted: Bool) async throws -> Channel {
        var body: [String: JSONValue] = ["target_user_ids": .array(try userIDs.map { .string(try segment($0)) })]
        if encrypted { body["encrypted"] = .bool(true) }
        let channel: Channel = try await api.write("api/dms", body: body)
        if encrypted, channel.encrypted != true { throw MessageEncryptionError.downgraded }
        return channel
    }

    func updateChannel(id: String, name: String?, description: String) async throws -> Channel {
        let version = sync?.channelVersion(id)
        var body: [String: JSONValue] = ["description": .string(description)]
        if let name { body["channel_name"] = .string(name) }
        let updated: Channel = try await api.write("api/channels/\(try segment(id))", method: "PATCH", body: body)
        try sync?.check()
        if let version, sync?.channelVersion(id) != version { throw CancellationError() }
        return updated
    }

    func archiveChannel(id: String) async throws -> Channel {
        let version = sync?.channelVersion(id)
        let updated: Channel = try await api.write("api/channels/\(try segment(id))/archive", body: [String: JSONValue]())
        try sync?.check()
        if let version, sync?.channelVersion(id) != version { throw CancellationError() }
        return updated
    }

    func dmMemberships() async throws -> [ChannelMembership] {
        try await shapes.snapshot("api/shapes/channel_users", as: ChannelMembership.self)
    }

    func joinChannel(id: String) async throws {
        _ = try await api.data("api/channels/\(try segment(id))/join", method: "POST")
    }

    private func segment(_ id: String) throws -> String {
        guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
        return id
    }
}

/// `GET /api/threads/{id}` の参加状態。返信の可否の判定に必要な項目だけを読む。
struct ThreadMembership: Decodable, Sendable, Equatable {
    /// `joined`・`left`・`none`
    let membership: String
    let can_participate: Bool
    let thread_archive_at: String?
}
