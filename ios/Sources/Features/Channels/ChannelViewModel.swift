import Foundation
import Observation

@Observable @MainActor
final class ChannelViewModel {
    private let repository: any WorkspaceRepository
    init(repository: any WorkspaceRepository) { self.repository = repository }
    var me: Person? { repository.state.me }
    var channels: [Channel] { repository.state.channels }
    var memberships: [Membership] { repository.state.memberships }
    var people: [Person] { repository.state.people }
    var unread: Set<String> { repository.state.unread }

    func person(_ id: String) -> Person? { people.first { $0.id == id } ?? (me?.id == id ? me : nil) }
    func name(_ id: String) -> String { person(id)?.display_name ?? "メンバー" }
    func joined(_ id: String) -> Bool { memberships.contains { $0.channel_id == id && $0.user_id == me?.id } }
    func channel(_ id: String) -> Channel? { channels.first { $0.id == id && $0.deleted_at == nil } }
    func canManage(_ channel: Channel) -> Bool { ChannelPolicy.canManage(channel, user: me) }
    func canInvite(_ channel: Channel) -> Bool { ChannelPolicy.canInvite(channel, user: me, joined: joined(channel.id)) }

    func title(_ channel: Channel) -> String {
        guard channel.is_dm else { return channel.channel_name }
        let names = memberships.filter { $0.channel_id == channel.id && $0.user_id != me?.id }.map { name($0.user_id) }
        return names.isEmpty ? "ダイレクトメッセージ" : names.sorted().joined(separator: ", ")
    }

    func createChannel(name: String, description: String, isPrivate: Bool) async throws -> Channel {
        try validate(name: name, description: description)
        return try await repository.createChannel(name: name, description: description, isPrivate: isPrivate)
    }

    func startDM(_ person: Person) async throws -> Channel {
        if let existing = channels.first(where: { channel in
            channel.is_dm && memberships.filter { $0.channel_id == channel.id }.count == 2 &&
                memberships.contains { $0.channel_id == channel.id && $0.user_id == person.id } && joined(channel.id)
        }) { return existing }
        return try await repository.startDM(person)
    }

    func join(_ channel: Channel) async throws { try await repository.join(channel) }
    func leave(_ channel: Channel) async throws { try await repository.leave(channel) }
    func invite(_ person: Person, to channel: Channel) async throws { try await repository.invite(person, to: channel) }
    func setArchived(_ channel: Channel, archived: Bool) async throws { try await repository.setArchived(channel, archived: archived) }
    func updateChannel(_ channel: Channel, name: String, description: String) async throws {
        try validate(name: name, description: description)
        try await repository.updateChannel(channel, name: name, description: description)
    }
    private func validate(name: String, description: String) throws {
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              name.unicodeScalars.count <= 80, description.unicodeScalars.count <= 250 else {
            throw ZooError.auth("名前は1〜80文字、説明は250文字以内で入力してください。")
        }
    }
}
