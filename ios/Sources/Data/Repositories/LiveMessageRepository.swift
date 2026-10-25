import Foundation

@MainActor
final class LiveMessageRepository: MessageRepository {
    let state: ConversationState
    var onAccessDenied: (() -> Void)?
    var onSessionExpired: (() -> Void)?
    private let session: RepositorySession
    private let userID: String
    private var streams: [ShapeStream] = []
    private var generation = UUID()
    private var accessGeneration = UUID()

    init(channelID: String, userID: String, session: RepositorySession) {
        state = ConversationState(channelID: channelID)
        self.session = session
        self.userID = userID
    }

    func start() {
        stop()
        state.phase = state.timeline.isEmpty ? .loading : .resyncing
        state.reactionError = nil
        streams = [
            subscribe("messages", type: Message.self) { [weak self] rows, ready in
                self?.state.receive(rows)
                self?.state.phase = ready ? .ready : .resyncing
            },
            subscribe("message_reactions", type: Reaction.self) { [weak self] rows, _ in self?.state.reactions = rows }
        ]
    }

    func stop() {
        generation = UUID()
        streams.forEach { $0.stop() }
        streams = []
        state.releaseReceivedHistory()
    }

    func reset() {
        stop()
        accessGeneration = UUID()
        state.clear()
        state.phase = .idle
    }

    private func subscribe<T: Decodable>(_ shape: String, type: T.Type,
                                         receive: @escaping @MainActor ([T], Bool) -> Void) -> ShapeStream {
        let stream = ShapeStream()
        let epoch = generation
        var snapshotReady = false
        stream.start(api: session.api, shape: shape, channelID: state.channelID, update: { [weak self] snapshot in
            guard let self, epoch == self.generation else { return }
            try self.session.check()
            receive(try snapshot.decode(type), snapshot.ready)
            snapshotReady = snapshot.ready
        }, failure: { [weak self] error in
            guard let self, epoch == self.generation else { return }
            if case ZooError.expiredSession = error { self.onSessionExpired?(); return }
            if case ZooError.rejected(let code, _) = error, code == 403 || code == 404 {
                self.reset()
                self.state.phase = .accessDenied
                self.onAccessDenied?()
            } else if shape == "messages" { self.state.phase = .failed(error.localizedDescription) }
            else { self.state.reactionError = error.localizedDescription }
        }, recovered: { [weak self] in
            guard let self, epoch == self.generation, snapshotReady else { return }
            if shape == "message_reactions" { self.state.reactionError = nil }
            else { self.state.phase = .ready }
        })
        return stream
    }

    func send(id: String, text: String, parentID: String?) async throws {
        let epoch = accessGeneration
        var body: [String: JSONValue] = ["id": .string(id), "channel_id": .string(state.channelID), "content": .string(text)]
        if let parentID { body["parent_id"] = .string(parentID) }
        let message: Message = try await session.write("api/messages", body: body)
        guard epoch == accessGeneration else { throw CancellationError() }
        state.acknowledge(message)
    }

    func edit(_ message: Message, content: String) async throws {
        let _: Message = try await session.write("api/messages/\(message.id)", method: "PATCH", body: ["content": .string(content)])
    }

    func delete(_ message: Message) async throws {
        let _: Message = try await session.write("api/messages/\(message.id)", method: "DELETE")
    }

    func react(_ message: Message, emoji: String) async throws {
        let existing = state.reactions.contains { $0.message_id == message.id && $0.user_id == userID && $0.emoji == emoji }
        if existing {
            let _: JSONValue = try await session.write("api/messages/\(message.id)/reactions", method: "DELETE", query: ["emoji": emoji])
        } else {
            let _: Reaction = try await session.write("api/messages/\(message.id)/reactions", body: ["emoji": .string(emoji)])
        }
    }
}
