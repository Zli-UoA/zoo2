#if DEBUG
import Foundation

@MainActor
final class PreviewPushReceiptStore: PushReceiptStore {
    private var receipt: PushReceipt?
    func load() throws -> PushReceipt? { receipt }
    func save(_ receipt: PushReceipt?) throws { self.receipt = receipt }
}

// UI テストは本番の認証・通知・投稿に触れず、実際の SwiftUI 画面を操作する。
@MainActor
final class PreviewChatClient: ChatClient {
    let serverURL = "https://ui.test.invalid"
    static let channelID = "11111111-1111-4111-8111-111111111111"
    static let messageID = "22222222-2222-4222-8222-222222222222"
    let currentUser = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")
    private let colleague = Person(id: "44444444-4444-4444-8444-444444444444", user_name: "sakura", display_name: "さくら")
    private var allChannels: [Channel] = [
        Channel(id: channelID, channel_name: "一般", description: "日々の話題やお知らせを共有しましょう。", is_private: false, is_dm: false),
        Channel(id: "55555555-5555-4555-8555-555555555555", channel_name: "開発", description: "ものづくりの相談はこちら。", is_private: false, is_dm: false),
        Channel(id: "77777777-7777-4777-8777-777777777777", channel_name: "暗号化", description: "暗号化された会話", is_private: true, is_dm: false, encrypted: true)
    ]
    private var allMessages: [Message] = []
    private var failNextSend: Bool
    private var incomingAt: ContinuousClock.Instant?

    init(failNextSend: Bool = ProcessInfo.processInfo.arguments.contains("--uitesting-send-failure")) {
        self.failNextSend = failNextSend
        let welcomeReaction = MessageReaction(id: "88888888-8888-4888-8888-888888888888", message_id: Self.messageID,
                                              user_id: colleague.id, emoji: "👍")
        allMessages = [
            Message(id: Self.messageID, user_id: colleague.id, channel_id: Self.channelID,
                    content: "こんにちは！ Zoo の iOS 版へようこそ。", created_at: "2026-11-18T09:00:00Z",
                    display_data: MessageDisplayData(reactions: [welcomeReaction])),
            Message(id: "66666666-6666-4666-8666-666666666666", user_id: currentUser.id, channel_id: Self.channelID,
                    content: "SwiftUI で、いつもの会話がもっと身近に。", created_at: "2026-11-18T09:01:00Z",
                    display_data: MessageDisplayData(reactions: []))
        ]
        if ProcessInfo.processInfo.arguments.contains("--uitesting-long-history") {
            allMessages = (1...60).map { index in
                Message(id: UUID().uuidString.lowercased(), user_id: colleague.id, channel_id: Self.channelID,
                        content: "履歴 \(index)", created_at: Date(timeIntervalSince1970: 1_795_000_000 + Double(index)).ISO8601Format(),
                        display_data: MessageDisplayData(reactions: []))
            }
        }
    }

    func channels() async throws -> [Channel] { allChannels }
    func channel(id: String) async throws -> Channel {
        guard let value = allChannels.first(where: { $0.id == id }) else { throw ZooError.rejected(404, "チャンネルがありません。") }
        return value
    }
    func people() async throws -> [Person] { [currentUser, colleague] }
    func person(id: String) async throws -> Person {
        guard let person = [currentUser, colleague].first(where: { $0.id == id }) else { throw ZooError.rejected(404, "ユーザーがいません。") }
        return person
    }
    func readStates() async throws -> [ChannelReadState] { [ChannelReadState(channel_id: Self.channelID, has_unread: true, last_read_at: nil)] }
    func emojis() async throws -> [WorkspaceEmoji] {
        [WorkspaceEmoji(name: "sakura", url: "", category: "動物園", aliases: ["hana"])]
    }

    func emojiAccounts() async throws -> [EmojiAccount] {
        [EmojiAccount(username: "sakura", display_name: "さくら", avatar_url: nil),
         EmojiAccount(username: "zoo", display_name: "動物園", avatar_url: nil)]
    }

    func observeReactions(channelID: String, receive: @MainActor ([MessageReaction]?) -> Void) async throws {
        while !Task.isCancelled {
            receive(allMessages.filter { $0.channel_id == channelID }.flatMap { $0.display_data?.reactions ?? [] })
            try await Task.sleep(for: .milliseconds(200))
        }
    }

    func addReaction(messageID: String, emoji: String) async throws -> MessageReaction {
        guard let index = allMessages.firstIndex(where: { $0.id == messageID }) else { throw ZooError.invalidResponse }
        var reactions = allMessages[index].display_data?.reactions ?? []
        if let existing = reactions.first(where: { $0.user_id == currentUser.id && $0.emoji == emoji }) { return existing }
        let row = MessageReaction(id: UUID().uuidString.lowercased(), message_id: messageID, user_id: currentUser.id, emoji: emoji)
        reactions.append(row)
        allMessages[index].display_data = MessageDisplayData(reactions: reactions)
        return row
    }

    func removeReaction(messageID: String, emoji: String) async throws {
        guard let index = allMessages.firstIndex(where: { $0.id == messageID }) else { throw ZooError.invalidResponse }
        allMessages[index].display_data?.reactions?.removeAll { $0.user_id == currentUser.id && $0.emoji == emoji }
    }

    func observeMessages(channelID: String, receive: @MainActor ([Message]?) -> Void) async throws {
        if ProcessInfo.processInfo.arguments.contains("--uitesting-long-history") {
            try await Task.sleep(for: .milliseconds(300))
        }
        while !Task.isCancelled {
            if let incomingAt, ContinuousClock.now >= incomingAt {
                self.incomingAt = nil
                allMessages.append(Message(id: UUID().uuidString.lowercased(), user_id: colleague.id, channel_id: Self.channelID,
                                           content: "別の端末からの新着", created_at: Date().ISO8601Format(),
                                           display_data: MessageDisplayData(reactions: [])))
            }
            receive(allMessages.filter { $0.channel_id == channelID })
            try await Task.sleep(for: .milliseconds(200))
        }
    }
    func send(id: String, channelID: String, content: String, parentID: String?) async throws -> Message {
        if failNextSend { failNextSend = false; throw URLError(.notConnectedToInternet) }
        if let existing = allMessages.first(where: { $0.id == id }) { return existing }
        let message = Message(id: id, user_id: currentUser.id, channel_id: channelID, content: content,
                              created_at: Date().ISO8601Format(), parent_id: parentID, display_data: MessageDisplayData(reactions: []))
        allMessages.append(message)
        if parentID != nil, ProcessInfo.processInfo.arguments.contains("--uitesting-live-message") {
            incomingAt = ContinuousClock.now.advanced(by: .seconds(5))
        }
        return message
    }
    func edit(id: String, content: String) async throws -> Message {
        guard let index = allMessages.firstIndex(where: { $0.id == id }) else { throw ZooError.invalidResponse }
        allMessages[index].content = content
        allMessages[index].updated_at = Date().ISO8601Format()
        return allMessages[index]
    }
    func delete(id: String) async throws { allMessages.removeAll { $0.id == id } }
    func markRead(channelID: String, messageID: String) async throws { }
    func createChannel(id: String, name: String, description: String, isPrivate: Bool) async throws -> Channel {
        let channel = Channel(id: id, channel_name: name, description: description, is_private: isPrivate, is_dm: false)
        allChannels.append(channel)
        return channel
    }
    func directMessage(userID: String) async throws -> Channel {
        let channel = Channel(id: UUID().uuidString.lowercased(), channel_name: "dm:\(currentUser.id):\(userID)", is_private: true, is_dm: true)
        allChannels.append(channel)
        return channel
    }
}
#endif
