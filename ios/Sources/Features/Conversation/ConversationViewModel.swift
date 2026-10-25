import Foundation
import Observation

@Observable @MainActor
final class ConversationViewModel {
    let state: ConversationState
    private let repository: any MessageRepository
    private let workspace: any WorkspaceRepository
    private let userID: String
    var drafts: [String: String] = [:]
    private(set) var sending: Set<String> = []
    private var attempts: [String: (id: String, text: String)] = [:]
    private var readers: Set<UUID> = []
    private var suspended = false
    private var valid = true
    private var generation = UUID()

    init(repository: any MessageRepository, workspace: any WorkspaceRepository, userID: String) {
        self.repository = repository
        self.workspace = workspace
        self.userID = userID
        self.state = repository.state
        repository.onAccessDenied = { [weak self] in self?.denyAccess() }
    }

    var timeline: [Message] { state.timeline }
    var reactions: [Reaction] { state.reactions }
    var replyCounts: [String: Int] { state.replyCounts }
    var loadingMessages: Bool { state.phase.isLoading }
    var accessDenied: Bool { state.phase == .accessDenied }
    var hasSyncError: Bool {
        guard !readers.isEmpty, !accessDenied else { return false }
        return state.phase.failure != nil || state.reactionError != nil
    }

    func open(reader: UUID) {
        guard valid else { return }
        let wasEmpty = readers.isEmpty
        readers.insert(reader)
        if wasEmpty && !suspended && !accessDenied { repository.start() }
    }

    func close(reader: UUID) {
        readers.remove(reader)
        if readers.isEmpty { repository.stop() }
    }

    func setActive(_ active: Bool) {
        suspended = !active
        if active && !readers.isEmpty && !accessDenied { repository.start() }
        else { repository.stop() }
    }

    func reconcileChannels() {
        let phase = workspace.state.channelPhase
        guard phase == .ready || phase == .accessDenied else { return }
        let available = phase == .ready && workspace.state.channels.contains { $0.id == state.channelID && $0.deleted_at == nil }
        if !available { denyAccess() }
        else if accessDenied {
            state.phase = .idle
            if !suspended && !readers.isEmpty { repository.start() }
        }
    }

    private func denyAccess() {
        generation = UUID()
        repository.reset()
        state.phase = .accessDenied
        drafts = [:]; attempts = [:]; sending = []
    }

    func reset() {
        valid = false
        generation = UUID()
        readers = []; drafts = [:]; attempts = [:]; sending = []
        repository.reset()
    }

    func send(_ text: String, parentID: String?) async throws {
        try MessagePolicy.validate(text)
        guard valid, !accessDenied else { throw CancellationError() }
        let key = parentID ?? "root"
        guard !sending.contains(key) else { return }
        sending.insert(key)
        let epoch = generation
        defer { if epoch == generation { sending.remove(key) } }
        if attempts[key]?.text != text { attempts[key] = (UUID().uuidString.lowercased(), text) }
        let id = attempts[key]!.id
        func replicated() -> Bool { timeline.contains { $0.id == id && $0.user_id == userID && $0.content == text } }
        if !replicated() {
            do { try await repository.send(id: id, text: text, parentID: parentID) }
            catch {
                guard epoch == generation else { throw CancellationError() }
                if !replicated() { throw error }
            }
        }
        guard epoch == generation else { throw CancellationError() }
        attempts.removeValue(forKey: key)
        if drafts[key] == text { drafts[key] = "" }
    }

    func edit(_ message: Message, content: String) async throws {
        try MessagePolicy.validate(content)
        try await repository.edit(message, content: content)
    }
    func delete(_ message: Message) async throws { try await repository.delete(message) }
    func react(_ message: Message, emoji: String) async throws { try await repository.react(message, emoji: emoji) }
    func markRead(messageID: String) async throws { try await workspace.markRead(state.channelID, messageID: messageID) }
}
