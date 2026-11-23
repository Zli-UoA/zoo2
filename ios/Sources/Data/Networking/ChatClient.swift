import Foundation

@MainActor
protocol ChatClient: InboxClient, CallClient, TypingClient {
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
    func readState(channelID: String, allowCached: Bool) async throws -> ChannelReadState
    func attachmentURL(id: String) async throws -> URL
    func attachmentData(from url: URL) async throws -> Data
    func attachmentData(_ attachment: MessageAttachment, channelID: String?, reload: Bool) async throws -> Data
    func attachmentFile(_ attachment: MessageAttachment, channelID: String?) async throws -> AttachmentFile
    func readStates(allowCached: Bool) async throws -> [ChannelReadState]
    func observeMessages(channelID: String, receive: @MainActor ([Message]?) -> Void) async throws
    func observeMessageFeed(channelID: String, request: MessageFeedRequest, receive: @MainActor (MessageFeedSnapshot) -> Void) async throws
    func cachedWorkspace() async throws -> CachedWorkspace?
    func observeAttachments(channelID: String, receive: @MainActor ([MessageAttachment]?) -> Void) async throws
    func observeReactions(channelID: String, receive: @MainActor ([MessageReaction]?) -> Void) async throws
    func addReaction(messageID: String, emoji: String) async throws -> MessageReaction
    func removeReaction(messageID: String, emoji: String) async throws
    func emojis() async throws -> [WorkspaceEmoji]
    func emojiAccounts() async throws -> [EmojiAccount]
    func upload(_ attachment: DraftAttachment, channelID: String) async throws -> String
    func send(id: String, channelID: String, content: String, parentID: String?, attachmentIDs: [String], sensitiveAttachmentIDs: [String]) async throws -> Message
    func edit(id: String, content: String) async throws -> Message
    func delete(id: String) async throws
    func markRead(channelID: String, messageID: String) async throws
    func markChannelRead(channelID: String) async throws -> ChannelReadState
    func createChannel(id: String, name: String, description: String, isPrivate: Bool) async throws -> Channel
    func updateChannel(id: String, name: String?, description: String) async throws -> Channel
    func archiveChannel(id: String) async throws -> Channel
    func directMessage(userIDs: [String]) async throws -> Channel
    func dmMemberships() async throws -> [ChannelMembership]
    func joinChannel(id: String) async throws
}

struct CachedWorkspace: Sendable {
    let channels: [Channel]
    let people: [Person]
    let readStates: [ChannelReadState]
}

extension ChatClient {
    func readState(channelID: String) async throws -> ChannelReadState {
        try await readState(channelID: channelID, allowCached: true)
    }
    func readStates() async throws -> [ChannelReadState] {
        try await readStates(allowCached: true)
    }
    func attachmentData(_ attachment: MessageAttachment, channelID: String? = nil, reload: Bool = false) async throws -> Data {
        let url = try await attachmentURL(id: attachment.id)
        try Task.checkCancellation()
        return try await attachmentData(from: url)
    }
    func cachedWorkspace() async throws -> CachedWorkspace? { nil }
    func attachmentFile(_ attachment: MessageAttachment, channelID: String?) async throws -> AttachmentFile {
        try await AttachmentFile.write(attachmentData(attachment, channelID: channelID, reload: false))
    }
    func observeMessageFeed(channelID: String, request: MessageFeedRequest, receive: @MainActor (MessageFeedSnapshot) -> Void) async throws {
        try await observeMessages(channelID: channelID) { receive(MessageFeedSnapshot(messages: $0)) }
    }
}
