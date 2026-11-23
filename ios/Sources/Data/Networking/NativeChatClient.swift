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
        let channels = try await shapes.snapshot("api/shapes/channels", as: Channel.self)
        let previous = try await sync.load("workspace:channels", as: [Channel].self) ?? []
        try sync.checkRevocationVersion(version)
        let available = Dictionary(channels.filter { $0.deleted_at == nil }.map { ($0.id, $0) },
                                   uniquingKeysWith: { _, new in new })
        for channel in previous where channel.deleted_at == nil {
            let current = available[channel.id]
            if current == nil || (channel.encrypted != true && current?.encrypted == true) {
                version = try await sync.revokeChannel(channel.id, retaining: current)
                try sync.checkRevocationVersion(version)
            }
        }
        try await sync.storeRecovery(channels, key: "workspace:channels", revocationVersion: version)
        return channels
    }
    func channel(id: String) async throws -> Channel {
        let version = sync?.channelVersion(id)
        do {
            let channel: Channel = try await api.get("api/channels/\(try segment(id))")
            if let sync, let version {
                try sync.check()
                guard sync.channelVersion(id) == version else { throw CancellationError() }
                if channel.deleted_at == nil, channel.encrypted != true {
                    try await sync.authorizeChannel(id, version: version)
                } else { try await sync.revokeChannel(id, retaining: channel.deleted_at == nil ? channel : nil) }
                try sync.check()
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
        guard let channels = try await sync.load("workspace:channels", as: [Channel].self),
              let people = try await sync.load("workspace:people", as: [Person].self) else { return nil }
        let states = try await sync.load("rest:api/channel-read-states", as: [ChannelReadState].self) ?? []
        try sync.checkRevocationVersion(version)
        return CachedWorkspace(channels: channels, people: people, readStates: states)
    }

    func cachedGet<T: Codable & Sendable>(_ path: String, query: [String: String] = [:],
                                         channelID: String? = nil, allowCached: Bool = true) async throws -> T {
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
            if allowCached, isOfflineFailure(error), let value = try await sync?.load(key, as: T.self) {
                try sync?.check(channelID: channelID, channelVersion: version)
                try sync?.checkRevocationVersion(revocationVersion)
                return value
            }
            throw error
        }
    }

    func channelPins() async throws -> ChannelPins { try await cachedGet("api/channel-pins") }
    func setChannelPinned(id: String, pinned: Bool, expectedVersion: Int64) async throws -> ChannelPins {
        let id = try segment(id)
        if pinned {
            return try await api.write("api/channel-pins", body: ["channel_id": .string(id), "expected_version": .number(Double(expectedVersion))])
        }
        return try await api.write("api/channel-pins/\(id)", method: "DELETE", query: ["expected_version": String(expectedVersion)])
    }
    func members(channelID: String) async throws -> [ChannelMembership] {
        try await shapes.snapshot("api/shapes/channel_users", query: ["channel_id": try segment(channelID)], as: ChannelMembership.self)
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
        guard let sync, let channelID, attachment.isImage || attachment.isPDF else {
            let url = try await attachmentURL(id: attachment.id)
            try Task.checkCancellation()
            return try await attachmentData(from: url)
        }
        _ = try segment(attachment.id)
        _ = try segment(channelID)
        let version = sync.channelVersion(channelID)
        try sync.check(channelID: channelID, channelVersion: version)
        let cached = try await sync.cache.loadAttachment(attachment, channelID: channelID, version: version,
                                                        reload: reload, lease: sync.lease)
        try sync.check(channelID: channelID, channelVersion: version)
        if let cached { return cached }
        let url: URL
        do { url = try await attachmentURL(id: attachment.id) }
        catch {
            try sync.check(channelID: channelID, channelVersion: version)
            if case ZooError.rejected(let status, _) = error, status == 403 || status == 404 {
                _ = try await sync.cache.loadAttachment(attachment, channelID: channelID, version: version, reload: true, lease: sync.lease)
            }
            throw error
        }
        try sync.check(channelID: channelID, channelVersion: version)
        let data = try await attachmentData(from: url)
        try sync.check(channelID: channelID, channelVersion: version)
        try await sync.cache.storeAttachment(data, attachment: attachment, channelID: channelID, version: version, lease: sync.lease)
        try sync.check(channelID: channelID, channelVersion: version)
        return data
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
        let result: AttachmentUpload = try await api.write("api/attachments/presign", body: [
            "channel_id": .string(try segment(channelID)), "file_name": .string(attachment.fileName),
            "file_size": .number(Double(attachment.byteCount)), "content_type": .string(attachment.contentType)
        ])
        let id = try segment(result.attachment_id)
        guard let url = URL(string: result.upload_url), url.scheme == "https", url.host != nil,
              url.user == nil, url.password == nil, url.fragment == nil else { throw ZooError.invalidResponse }
        try Task.checkCancellation()
        var request = URLRequest(url: url)
        request.httpMethod = "PUT"
        request.setValue(attachment.contentType, forHTTPHeaderField: "Content-Type")
        try await api.transport.upload(request, from: attachment.file.url)
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
                guard let parentID, try await rejoinThread(messageID: parentID) else {
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

    /// 参加が必要なスレッドへ参加する。参加できる状態でなければ false を返す。
    private func rejoinThread(messageID: String) async throws -> Bool {
        let path = "api/threads/\(try segment(messageID))"
        // 参加状態（`membership`・`can_participate`・`thread_archive_at`）だけが要る。参加者の一覧（人数に比例する
        // 読み取り）は要らないので、3つの呼び出しすべてで省く（サーバーは `participants=none` を受け付ける）。
        let lean = ["participants": "none"]
        // GET に本文を付けると URLSession が拒否するため、`write` ではなく本文なしの `data` で取得する。
        let status = try JSONDecoder().decode(Envelope<ThreadMembership>.self, from: await api.data(path, query: lean)).data
        guard status.can_participate else { return false }
        if status.thread_archive_at != nil {
            _ = try await api.data("\(path)/unarchive", method: "POST", query: lean)
        }
        if status.membership != "joined" { _ = try await api.data("\(path)/join", method: "POST", query: lean) }
        return true
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

    func createChannel(id: String, name: String, description: String, isPrivate: Bool) async throws -> Channel {
        try await api.write("api/channels", body: ["id": .string(try segment(id)), "channel_name": .string(name),
                                                  "description": .string(description), "is_private": .bool(isPrivate)])
    }

    func directMessage(userIDs: [String]) async throws -> Channel {
        try await api.write("api/dms", body: ["target_user_ids": .array(try userIDs.map { .string(try segment($0)) })])
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
