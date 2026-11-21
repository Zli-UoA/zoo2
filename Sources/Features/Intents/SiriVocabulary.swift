import Intents
import CoreSpotlight
import os

@MainActor
enum SiriVocabulary {
    private static var indexedRecipients: [ShortcutRecipient]?
    private static var indexedChannels: [ShortcutChannel]?
    private static var indexing: Task<Void, Never>?
    private static var updating: Task<Void, Never>?

    static func refreshAuthorization(_ workspace: WorkspaceStore?) {
        // Siriによるバックグラウンド起動では、未読込の一覧で登録済み宛先を消さない。
        if INPreferences.siriAuthorizationStatus() == .authorized, workspace?.hasLoaded != true { return }
        update(workspace)
    }

    static func update(_ workspace: WorkspaceStore?) {
        updating?.cancel()
        updating = nil
        guard INPreferences.siriAuthorizationStatus() == .authorized, let workspace else {
            apply(recipients: [], channels: [])
            return
        }
        updating = Task {
            let values = (try? await candidates(in: workspace)) ?? (recipients: [], channels: [])
            guard !Task.isCancelled else { return }
            apply(recipients: values.recipients, channels: values.channels)
        }
    }

    static func candidates(in workspace: WorkspaceStore) async throws -> (recipients: [ShortcutRecipient], channels: [ShortcutChannel]) {
        let actions = ShortcutActions(workspace: workspace)
        // 古いキャッシュからアーカイブ済み宛先を再登録しない。通信失敗時もチャンネルは登録しない。
        let channels = (try? await actions.channels()) ?? []
        return (try actions.recipients(in: Array(workspace.people.values)), channels)
    }

    private static func apply(recipients: [ShortcutRecipient], channels: [ShortcutChannel]) {
        // 端末の連絡先にないメンバーも認識させ、ログアウト時には古いアカウントの候補を消す。
        let vocabulary = recipients.map {
            INSpeakableString(vocabularyIdentifier: $0.id, spokenPhrase: $0.name, pronunciationHint: nil)
        }
        INVocabulary.shared().setVocabulary(NSOrderedSet(array: vocabulary), of: .contactName)
        let groups = channels.map {
            INSpeakableString(vocabularyIdentifier: $0.id, spokenPhrase: $0.name, pronunciationHint: nil)
        }
        INVocabulary.shared().setVocabulary(NSOrderedSet(array: groups), of: .contactGroupName)
        if #available(iOS 27.0, *), indexedRecipients != recipients || indexedChannels != channels {
            indexedRecipients = recipients
            indexedChannels = channels
            let previous = indexing
            previous?.cancel()
            // 更新を直列化し、ログアウト後に古い宛先の登録が完了する順序逆転を防ぐ。
            indexing = Task { @concurrent in
                await previous?.value
                guard !Task.isCancelled else { return }
                do {
                    let index = CSSearchableIndex.default()
                    // Zooの検索索引はSiri宛先だけ。型変更前や別アカウントの宛先もまとめて消す。
                    try await index.deleteAllSearchableItems()
                    guard !Task.isCancelled else { return }
                    try await index.indexAppEntities(recipients.map(ZooMessagePerson.init))
                    guard !Task.isCancelled else { return }
                    try await index.indexAppEntities(recipients.map(ZooRecipientEntity.init))
                    guard !Task.isCancelled else { return }
                    try await index.indexAppEntities(recipients.map(ZooPhonePerson.init))
                    guard !Task.isCancelled else { return }
                    try await index.indexAppEntities(channels.map(ZooChannelEntity.init))
                    guard !Task.isCancelled else { return }
                    Logger(subsystem: "works.zli.zoo", category: "Siri").notice("Siri向けの宛先登録を更新しました。メンバー: \(recipients.count)件、チャンネル: \(channels.count)件")
                } catch {
                    Logger(subsystem: "works.zli.zoo", category: "Siri").error("Siri向けの宛先登録に失敗しました。")
                    await MainActor.run {
                        if indexedRecipients == recipients, indexedChannels == channels {
                            indexedRecipients = nil
                            indexedChannels = nil
                        }
                    }
                }
            }
        }
    }
}
