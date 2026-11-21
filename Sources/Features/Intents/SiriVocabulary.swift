import Intents
import CoreSpotlight
import os

@MainActor
enum SiriVocabulary {
    private static var indexedRecipients: [ShortcutRecipient]?
    private static var indexing: Task<Void, Never>?

    static func refreshAuthorization(_ workspace: WorkspaceStore?) {
        // Siriによるバックグラウンド起動では、未読込の一覧で登録済み宛先を消さない。
        if INPreferences.siriAuthorizationStatus() == .authorized, workspace?.hasLoaded != true { return }
        update(workspace)
    }

    static func update(_ workspace: WorkspaceStore?) {
        let recipients: [ShortcutRecipient]
        if INPreferences.siriAuthorizationStatus() == .authorized, let workspace {
            recipients = (try? ShortcutActions(workspace: workspace).recipients(in: Array(workspace.people.values))) ?? []
        } else {
            recipients = []
        }
        // 端末の連絡先にないメンバーも認識させ、ログアウト時には古いアカウントの候補を消す。
        let vocabulary = recipients.map {
            INSpeakableString(vocabularyIdentifier: $0.id, spokenPhrase: $0.name, pronunciationHint: nil)
        }
        INVocabulary.shared().setVocabulary(NSOrderedSet(array: vocabulary), of: .contactName)
        // 旧ビルドが登録したチャンネル名をSiriの候補から削除する。
        INVocabulary.shared().setVocabulary(NSOrderedSet(), of: .contactGroupName)
        if #available(iOS 27.0, *), indexedRecipients != recipients {
            indexedRecipients = recipients
            let previous = indexing
            previous?.cancel()
            // 更新を直列化し、ログアウト後に古い宛先の登録が完了する順序逆転を防ぐ。
            indexing = Task { @concurrent in
                await previous?.value
                guard !Task.isCancelled else { return }
                do {
                    let index = CSSearchableIndex.default()
                    // 型変更前の宛先や廃止したチャンネルも、再生成する索引から消す。
                    try await index.deleteAllSearchableItems()
                    guard !Task.isCancelled else { return }
                    try await index.indexAppEntities(recipients.map(ZooMessagePerson.init))
                    guard !Task.isCancelled else { return }
                    try await index.indexAppEntities(recipients.map(ZooRecipientEntity.init))
                    guard !Task.isCancelled else { return }
                    try await index.indexAppEntities(recipients.map(ZooPhonePerson.init))
                    Logger(subsystem: "works.zli.zoo", category: "Siri").notice("Siri向けの宛先登録を更新しました。件数: \(recipients.count)")
                } catch {
                    Logger(subsystem: "works.zli.zoo", category: "Siri").error("Siri向けの宛先登録に失敗しました。")
                    await MainActor.run {
                        if indexedRecipients == recipients { indexedRecipients = nil }
                    }
                }
            }
        }
    }
}
