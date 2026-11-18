import Foundation

@MainActor
protocol ChatClient: Sendable {
    var serverURL: String { get }
    func channels() async throws -> [Channel]
    func channel(id: String) async throws -> Channel
    func people() async throws -> [Person]
    func person(id: String) async throws -> Person
    func readStates() async throws -> [ChannelReadState]
    func observeMessages(channelID: String, receive: @MainActor ([Message]?) -> Void) async throws
    func observeReactions(channelID: String, receive: @MainActor ([MessageReaction]?) -> Void) async throws
    func addReaction(messageID: String, emoji: String) async throws -> MessageReaction
    func removeReaction(messageID: String, emoji: String) async throws
    func emojis() async throws -> [WorkspaceEmoji]
    func emojiAccounts() async throws -> [EmojiAccount]
    func send(id: String, channelID: String, content: String, parentID: String?) async throws -> Message
    func edit(id: String, content: String) async throws -> Message
    func delete(id: String) async throws
    func markRead(channelID: String, messageID: String) async throws
    func createChannel(id: String, name: String, description: String, isPrivate: Bool) async throws -> Channel
    func directMessage(userID: String) async throws -> Channel
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

    func observeMessages(channelID: String, receive: @MainActor ([Message]?) -> Void) async throws {
        _ = try segment(channelID)
        try await ShapeStream(api: api).observe("api/shapes/messages", query: ["channel_id": channelID],
                                               as: Message.self, receive: receive)
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

    func send(id: String, channelID: String, content: String, parentID: String?) async throws -> Message {
        _ = try segment(id)
        _ = try segment(channelID)
        var body: [String: JSONValue] = ["id": .string(id), "channel_id": .string(channelID), "content": .string(content)]
        if let parentID { body["parent_id"] = .string(try segment(parentID)) }
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

    func directMessage(userID: String) async throws -> Channel {
        try await api.write("api/dms", body: ["target_user_ids": .array([.string(try segment(userID))])])
    }

    private func segment(_ id: String) throws -> String {
        guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
        return id
    }
}
