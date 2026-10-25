import Foundation
import Observation

@Observable @MainActor
final class WorkspaceContext {
    let channels: ChannelViewModel
    let notifications: NotificationsViewModel
    let profile: ProfileViewModel
    let repository: any WorkspaceRepository
    private let makeMessages: (String) -> any MessageRepository
    private(set) var conversations: [String: ConversationViewModel] = [:]
    var onSessionExpired: (() -> Void)?
    private var active = true

    init(repository: any WorkspaceRepository, makeMessages: @escaping (String) -> any MessageRepository) {
        self.repository = repository
        self.makeMessages = makeMessages
        channels = ChannelViewModel(repository: repository)
        notifications = NotificationsViewModel(repository: repository)
        profile = ProfileViewModel(repository: repository)
        repository.onChannelsChanged = { [weak self] in
            self?.conversations.values.forEach { $0.reconcileChannels() }
        }
        repository.onSessionExpired = { [weak self] in self?.onSessionExpired?() }
    }

    var hasSyncErrors: Bool {
        !repository.state.syncErrors.isEmpty || conversations.values.contains { $0.hasSyncError }
    }

    func conversation(_ channelID: String) -> ConversationViewModel {
        if let existing = conversations[channelID] { return existing }
        let messages = makeMessages(channelID)
        messages.onSessionExpired = { [weak self] in self?.onSessionExpired?() }
        let model = ConversationViewModel(repository: messages, workspace: repository, userID: repository.state.me?.id ?? "")
        model.setActive(active)
        model.reconcileChannels()
        conversations[channelID] = model
        return model
    }

    func setActive(_ value: Bool) {
        active = value
        if value { repository.start() } else { repository.stop() }
        conversations.values.forEach { $0.setActive(value) }
    }

    func reconnect() {
        guard active else { return }
        repository.start()
        conversations.values.forEach { $0.setActive(true) }
    }

    func reset() {
        conversations.values.forEach { $0.reset() }
        conversations = [:]
        repository.reset()
    }
}
