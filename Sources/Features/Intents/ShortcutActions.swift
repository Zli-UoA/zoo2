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

struct ShortcutChannel: Identifiable, Hashable, Sendable {
    let id: String
    let channelID: String
    let name: String

    static func matching(identifier: String?, name: String, in channels: [Self]) -> [Self] {
        if let identifier { return channels.filter { $0.id == identifier } }
        let query = normalizedName(name)
        guard !query.isEmpty else { return [] }
        let exact = channels.filter { normalizedName($0.name) == query }
        return exact.isEmpty ? channels.filter { normalizedName($0.name).contains(query) } : exact
    }

    static func normalizedName(_ name: String) -> String {
        var result = name.folding(options: .widthInsensitive, locale: .current)
            .folding(options: .caseInsensitive, locale: .current).precomposedStringWithCanonicalMapping
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if result.hasPrefix("#") { result.removeFirst() }
        return result.trimmingCharacters(in: .whitespacesAndNewlines)
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
        // Siriの宛先照合は同期済み一覧で行い、短時間のバックグラウンド実行で全件通信を始めない。
        // 実際に送信する直前にはresolveで相手の有効性をサーバーへ再確認する。
        if workspace.hasLoaded {
            return try recipients(in: Array(workspace.people.values))
        }
        if let cached = try await workspace.client.cachedWorkspace() {
            return try recipients(in: cached.people)
        }
        let people = try await workspace.client.people()
        return try recipients(in: people)
    }

    func recipients(in people: [Person]) throws -> [ShortcutRecipient] {
        try check()
        return people.filter { !$0.is_bot && !$0.is_deactivated && $0.id != workspace.user.id }.map {
            ShortcutRecipient(id: identifier($0.id), personID: $0.id, name: $0.name, username: $0.user_name)
        }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    func channels() async throws -> [ShortcutChannel] {
        try check()
        return try await channels(in: workspace.client.currentChannels())
    }

    func channelTarget(identifier: String, name: String) throws -> ShortcutChannel {
        try check()
        let prefix = self.identifier("channel|")
        guard identifier.hasPrefix(prefix) else {
            throw ZooError.auth("このチャンネルは別のアカウントのものです。宛先を選び直してください。")
        }
        let channelID = String(identifier.dropFirst(prefix.count))
        guard UUID(uuidString: channelID) != nil else { throw ZooError.invalidResponse }
        return ShortcutChannel(id: identifier, channelID: channelID, name: name)
    }

    func channels(in channels: [Channel]) throws -> [ShortcutChannel] {
        try check()
        return channels.filter { !$0.is_dm && $0.canPost }.map {
            ShortcutChannel(id: identifier("channel|\($0.id)"), channelID: $0.id, name: $0.channel_name)
        }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    func resolve(_ target: ShortcutChannel) async throws -> Channel {
        try check()
        guard target.id == identifier("channel|\(target.channelID)") else {
            throw ZooError.auth("このチャンネルは別のアカウントのものです。宛先を選び直してください。")
        }
        // 候補のキャッシュだけでは、アーカイブや閲覧権限の取り消しを検出できない。
        let channel = try await workspace.client.channel(id: target.channelID)
        try check()
        guard channel.id == target.channelID, !channel.is_dm, channel.canPost else {
            throw ZooError.auth("このチャンネルには投稿できません。宛先を選び直してください。")
        }
        // 検索・確認後の改名を、以前の名前への投稿として扱わない。
        guard target.name.isEmpty || ShortcutChannel.normalizedName(target.name) == ShortcutChannel.normalizedName(channel.channel_name) else {
            throw ZooError.auth("チャンネル名が変更されました。宛先を選び直してください。")
        }
        return channel
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

    @discardableResult
    func send(_ content: String, to recipient: ShortcutRecipient) async throws -> Message {
        try Self.validateMessage(content)
        let channel = try await directMessage(to: recipient)
        guard channel.canPost else { throw ZooError.auth("暗号化された会話にはSiriから送信できません。") }
        return try await send(content, in: channel)
    }

    @discardableResult
    func send(_ content: String, to target: ShortcutChannel) async throws -> Message {
        try Self.validateMessage(content)
        return try await send(content, in: resolve(target))
    }

    private func send(_ content: String, in channel: Channel) async throws -> Message {
        let message: Message
        do {
            message = try await workspace.client.send(id: UUID().uuidString.lowercased(), channelID: channel.id,
                                              content: content, parentID: nil, attachmentIDs: [], sensitiveAttachmentIDs: [])
        } catch {
            workspace.report(error)
            throw error
        }
        // サーバーで成立した送信を失敗に変えず、ログアウト後の画面遷移だけを抑止する。
        if workspace.active, !Task.isCancelled { workspace.addAndOpen(channel) }
        return message
    }

    func draft(_ content: String, to recipient: ShortcutRecipient?) async throws {
        try check()
        guard content.unicodeScalars.count <= 40_000 else {
            throw ZooError.auth("メッセージは40,000文字以内で入力してください。")
        }
        guard let recipient else {
            // 下書きスキーマは画面での編集に引き継ぐため、未指定の宛先をSiriで聞き返さない。
            if let sheet = workspace.presentedSheet {
                if content.isEmpty { return }
                guard case .draftDirectMessage(let existing) = sheet, existing.isEmpty || existing == content else {
                    throw ZooError.auth("入力中の画面があります。Zooで内容を確認してください。")
                }
                if existing == content { return }
            }
            workspace.selectedTab = .directMessages
            workspace.presentedSheet = .draftDirectMessage(content)
            return
        }
        let channel = try await directMessage(to: recipient)
        guard channel.canPost else { throw ZooError.auth("この会話には下書きを作成できません。") }
        let conversation = workspace.conversation(for: channel)
        let key = conversation.draftKey(nil)
        let existing = conversation.drafts[key] ?? ""
        guard content.isEmpty || existing.isEmpty || existing == content else {
            throw ZooError.auth("入力中の下書きがあります。Zooで内容を確認してください。")
        }
        if !content.isEmpty { conversation.drafts[key] = content }
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
