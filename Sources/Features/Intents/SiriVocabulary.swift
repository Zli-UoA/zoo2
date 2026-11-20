import Intents

@MainActor
enum SiriVocabulary {
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
    }
}
