import Foundation

@MainActor
protocol ChatClient: InboxClient, CallClient {
    var accountProfileURL: URL? { get }
    func publicProfile() async throws -> PublicProfile
    func updatePublicProfile(name: String, summary: String) async throws -> PublicProfile
    func channels() async throws -> [Channel]
    func joinedThreads(before: String?, through: String?) async throws -> ChannelThreadPage
    func channel(id: String) async throws -> Channel
    func people() async throws -> [Person]
    func person(id: String) async throws -> Person
    func channelPins() async throws -> ChannelPins
    func setChannelPinned(id: String, pinned: Bool, expectedVersion: Int64) async throws -> ChannelPins
    func members(channelID: String) async throws -> [ChannelMembership]
    func readState(channelID: String) async throws -> ChannelReadState
    func attachmentURL(id: String) async throws -> URL
    func attachmentData(from url: URL) async throws -> Data
    func readStates() async throws -> [ChannelReadState]
    func observeMessages(channelID: String, receive: @MainActor ([Message]?) -> Void) async throws
    func observeAttachments(channelID: String, receive: @MainActor ([MessageAttachment]?) -> Void) async throws
    func observeReactions(channelID: String, receive: @MainActor ([MessageReaction]?) -> Void) async throws
    func addReaction(messageID: String, emoji: String) async throws -> MessageReaction
    func removeReaction(messageID: String, emoji: String) async throws
    func emojis() async throws -> [WorkspaceEmoji]
    func emojiAccounts() async throws -> [EmojiAccount]
    func upload(_ attachment: DraftAttachment, channelID: String) async throws -> String
    func send(id: String, channelID: String, content: String, parentID: String?, attachmentIDs: [String]) async throws -> Message
    func edit(id: String, content: String) async throws -> Message
    func delete(id: String) async throws
    func markRead(channelID: String, messageID: String) async throws
    func createChannel(id: String, name: String, description: String, isPrivate: Bool) async throws -> Channel
    func directMessage(userIDs: [String]) async throws -> Channel
    func dmMemberships() async throws -> [ChannelMembership]
    func joinChannel(id: String) async throws
}

@MainActor
struct NativeChatClient: ChatClient {
    let api: APIClient
    var serverURL: String { api.settings.server }

    func channels() async throws -> [Channel] { try await api.get("api/channels") }
    func channel(id: String) async throws -> Channel { try await api.get("api/channels/\(try segment(id))") }
    func people() async throws -> [Person] {
        // REST のユーザー応答には avatar_url がないため、Web と同じ shape を使う。
        try await ShapeStream(api: api).snapshot("api/shapes/users", as: Person.self)
    }
    func person(id: String) async throws -> Person { try await api.get("api/users/\(try segment(id))") }
    func readStates() async throws -> [ChannelReadState] { try await api.get("api/channel-read-states") }

    func channelPins() async throws -> ChannelPins { try await api.get("api/channel-pins") }
    func setChannelPinned(id: String, pinned: Bool, expectedVersion: Int64) async throws -> ChannelPins {
        let id = try segment(id)
        if pinned {
            return try await api.write("api/channel-pins", body: ["channel_id": .string(id), "expected_version": .number(Double(expectedVersion))])
        }
        return try await api.write("api/channel-pins/\(id)", method: "DELETE", query: ["expected_version": String(expectedVersion)])
    }
    func members(channelID: String) async throws -> [ChannelMembership] {
        try await ShapeStream(api: api).snapshot("api/shapes/channel_users", query: ["channel_id": try segment(channelID)], as: ChannelMembership.self)
    }
    func readState(channelID: String) async throws -> ChannelReadState {
        try await api.get("api/channels/\(try segment(channelID))/read-state")
    }
    func attachmentURL(id: String) async throws -> URL {
        let result: Envelope<AttachmentDownload> = try await api.get("api/attachments/\(try segment(id))/url")
        guard let url = result.data.url else { throw ZooError.invalidResponse }
        return url
    }

    func attachmentData(from url: URL) async throws -> Data {
        guard DisplayEmoji.assetURL(url.absoluteString) != nil else { throw ZooError.invalidResponse }
        // 署名付きURLにはAPIの認証トークンを付けず、ストレージの401でログアウトさせない。
        let (data, response) = try await api.transport.send(URLRequest(url: url))
        try Task.checkCancellation()
        guard (200..<300).contains(response.statusCode) else {
            throw ZooError.rejected(response.statusCode, "添付ファイルを取得できませんでした。")
        }
        return data
    }

    func observeMessages(channelID: String, receive: @MainActor ([Message]?) -> Void) async throws {
        _ = try segment(channelID)
        try await ShapeStream(api: api).observe("api/shapes/messages", query: ["channel_id": channelID],
                                               as: Message.self, receive: receive)
    }

    func observeAttachments(channelID: String, receive: @MainActor ([MessageAttachment]?) -> Void) async throws {
        try await ShapeStream(api: api).observe("api/shapes/attachments", query: ["channel_id": try segment(channelID)], as: MessageAttachment.self, receive: receive)
    }

    func observeReactions(channelID: String, receive: @MainActor ([MessageReaction]?) -> Void) async throws {
        try await ShapeStream(api: api).observe("api/shapes/message_reactions", query: ["channel_id": try segment(channelID)],
                                               as: MessageReaction.self, receive: receive)
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

    func send(id: String, channelID: String, content: String, parentID: String?, attachmentIDs: [String] = []) async throws -> Message {
        _ = try segment(id)
        _ = try segment(channelID)
        var body: [String: JSONValue] = ["id": .string(id), "channel_id": .string(channelID), "content": .string(content)]
        if let parentID { body["parent_id"] = .string(try segment(parentID)) }
        if !attachmentIDs.isEmpty { body["attachment_ids"] = .array(try attachmentIDs.map { .string(try segment($0)) }) }
        return try await api.write("api/messages", body: body)
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

    func createChannel(id: String, name: String, description: String, isPrivate: Bool) async throws -> Channel {
        try await api.write("api/channels", body: ["id": .string(try segment(id)), "channel_name": .string(name),
                                                  "description": .string(description), "is_private": .bool(isPrivate)])
    }

    func directMessage(userIDs: [String]) async throws -> Channel {
        try await api.write("api/dms", body: ["target_user_ids": .array(try userIDs.map { .string(try segment($0)) })])
    }

    func dmMemberships() async throws -> [ChannelMembership] {
        try await ShapeStream(api: api).snapshot("api/shapes/channel_users", as: ChannelMembership.self)
    }

    func joinChannel(id: String) async throws {
        _ = try await api.data("api/channels/\(try segment(id))/join", method: "POST")
    }

    private func segment(_ id: String) throws -> String {
        guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
        return id
    }
}
