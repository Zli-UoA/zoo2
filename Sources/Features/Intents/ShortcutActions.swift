import Foundation

struct ShortcutRecipient: Identifiable, Hashable, Sendable {
    let id: String
    let personID: String
    let name: String
    let username: String

    func matches(_ text: String) -> Bool {
        let query = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return !query.isEmpty && (name.localizedStandardContains(query) || username.localizedStandardContains(query))
    }
}

@MainActor
struct ShortcutActions {
    let workspace: WorkspaceStore

    private func check() throws {
        try Task.checkCancellation()
        guard workspace.active, !workspace.user.is_deactivated else { throw ZooError.expiredSession }
    }

    private func identifier(_ personID: String) -> String {
        "\(workspace.client.serverURL)|\(workspace.user.id)|\(personID)"
    }

    func recipients() async throws -> [ShortcutRecipient] {
        try check()
        let people = try await workspace.client.people()
        return try recipients(in: people)
    }

    func recipients(in people: [Person]) throws -> [ShortcutRecipient] {
        try check()
        return people.filter { !$0.is_bot && !$0.is_deactivated && $0.id != workspace.user.id }.map {
            ShortcutRecipient(id: identifier($0.id), personID: $0.id, name: $0.name, username: $0.user_name)
        }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    func resolve(_ recipient: ShortcutRecipient) async throws -> Person {
        try check()
        // 保存済みのショートカットを別アカウントで実行しても送信先を引き継がない。
        guard recipient.id == identifier(recipient.personID) else {
            throw ZooError.auth("この宛先は別のアカウントのものです。宛先を選び直してください。")
        }
        let person = try await workspace.client.person(id: recipient.personID)
        try check()
        guard person.id == recipient.personID, !person.is_bot, !person.is_deactivated, person.id != workspace.user.id else {
            throw ZooError.auth("このメンバーには送信・発信できません。宛先を選び直してください。")
        }
        return person
    }

    private func directMessage(to recipient: ShortcutRecipient) async throws -> Channel {
        let person = try await resolve(recipient)
        let channel = try await workspace.client.directMessage(userIDs: [person.id])
        try check()
        guard channel.is_dm, channel.deleted_at == nil, channel.archive_at == nil else {
            throw ZooError.auth("この会話は利用できません。")
        }
        return channel
    }

    nonisolated static func validateMessage(_ content: String) throws {
        guard !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, content.unicodeScalars.count <= 40_000 else {
            throw ZooError.auth("メッセージは1文字以上、40,000文字以内で入力してください。")
        }
    }

    func send(_ content: String, to recipient: ShortcutRecipient) async throws {
        try Self.validateMessage(content)
        let channel = try await directMessage(to: recipient)
        guard channel.canPost else { throw ZooError.auth("暗号化された会話にはSiriから送信できません。") }
        do {
            _ = try await workspace.client.send(id: UUID().uuidString.lowercased(), channelID: channel.id,
                                              content: content, parentID: nil, attachmentIDs: [], sensitiveAttachmentIDs: [])
        } catch {
            workspace.report(error)
            throw error
        }
        try check()
        workspace.addAndOpen(channel)
    }

    func call(_ recipient: ShortcutRecipient) async throws {
        try check()
        guard workspace.calls.channel == nil, workspace.systemCalls?.isManagingCall != true else {
            throw ZooError.auth("別の通話や着信を終了してから、もう一度お試しください。")
        }
        let channel = try await directMessage(to: recipient)
        workspace.addAndOpen(channel)
        try await workspace.calls.startAndWait(channel: channel)
        try check()
    }
}
