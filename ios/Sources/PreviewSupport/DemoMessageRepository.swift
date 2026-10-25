import Foundation

@MainActor
final class DemoMessageRepository: MessageRepository {
    let state: ConversationState
    var onAccessDenied: (() -> Void)?
    var onSessionExpired: (() -> Void)?
    private let userID: String
    private var messages: [Message]

    init(channelID: String, userID: String, messages: [Message] = [], reactions: [Reaction] = []) {
        self.state = ConversationState(channelID: channelID)
        self.userID = userID
        self.messages = messages.filter { $0.channel_id == channelID }
        state.receive(self.messages)
        let ids = Set(self.messages.map(\.id))
        state.reactions = reactions.filter { ids.contains($0.message_id) }
        state.phase = .ready
    }

    func start() { state.phase = .ready }
    func stop() {}
    func reset() { messages = []; state.clear(); state.phase = .idle }

    func send(id: String, text: String, parentID: String?) async throws {
        if !messages.contains(where: { $0.id == id }) {
            messages.append(Message(id: id, user_id: userID, channel_id: state.channelID,
                                    content: text, created_at: ZooDate.now(), parent_id: parentID))
        }
        state.receive(messages)
    }

    func edit(_ message: Message, content: String) async throws {
        if let index = messages.firstIndex(where: { $0.id == message.id }) {
            messages[index].content = content
            messages[index].updated_at = ZooDate.now()
            state.receive(messages)
        }
    }

    func delete(_ message: Message) async throws {
        if let index = messages.firstIndex(where: { $0.id == message.id }) {
            messages[index].deleted_at = ZooDate.now()
            state.receive(messages)
        }
    }

    func react(_ message: Message, emoji: String) async throws {
        if let existing = state.reactions.first(where: { $0.message_id == message.id && $0.user_id == userID && $0.emoji == emoji }) {
            state.reactions.removeAll { $0.id == existing.id }
        } else {
            state.reactions.append(Reaction(id: UUID().uuidString, message_id: message.id, user_id: userID, emoji: emoji))
        }
    }
}
