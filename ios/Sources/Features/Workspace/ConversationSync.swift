import SwiftUI

struct ConversationSync: ViewModifier {
    let workspace: WorkspaceStore
    @Environment(\.scenePhase) private var scenePhase

    private struct Authors: Equatable {
        let channelID: String
        let ids: Set<String>
    }

    private struct TypingScope: Equatable {
        let channelID: String
        let parentID: String?
    }

    private struct MessageScope: Equatable {
        let channelID: String
        let requestID: UUID
        let parentID: String?
        let focusID: String?
    }

    private var typingScope: TypingScope? {
        guard let store = activeConversation, store.canPost, let route = workspace.path.last else { return nil }
        switch route {
        case .channel(let id): return TypingScope(channelID: id, parentID: nil)
        case .thread(let id, let parentID, _):
            guard store.message(id: parentID) != nil else { return nil }
            return TypingScope(channelID: id, parentID: parentID)
        }
    }

    private var activeConversation: ConversationStore? {
        guard scenePhase == .active, let id = workspace.path.last?.channelID,
              let channel = workspace.channels.first(where: { $0.id == id }) else { return nil }
        return workspace.conversation(for: channel)
    }

    private var feedParentID: String? {
        if case .thread(_, let parent, _) = workspace.path.last { return parent }
        return nil
    }

    func body(content: Content) -> some View {
        let store = activeConversation
        let readable = store.flatMap { !$0.blocked && $0.channel.encrypted != true ? $0 : nil }
        let attachments = readable.flatMap { $0.needsAttachmentStream ? $0 : nil }
        let reactions = readable.flatMap { $0.needsReactionStream ? $0 : nil }
        let authors = readable.map { Authors(channelID: $0.channel.id, ids: Set($0.messages.map(\.user_id))) }
        let needsEmojiCatalog = scenePhase == .active && (store != nil || workspace.selectedTab == .notifications)
        let typing = typingScope
        let messageScope = readable.map { MessageScope(channelID: $0.channel.id, requestID: $0.syncRequest,
                                                       parentID: feedParentID, focusID: $0.focusedMessageID) }
        content
            .task(id: typing) {
                if let typing, let store { await store.typing.run(parentID: typing.parentID) }
            }
            .task(id: messageScope) {
                guard let readable, let messageScope else { return }
                readable.selectFeed(parentID: messageScope.parentID)
                await readable.run()
            }
            .task(id: readable?.channel.id) { if let readable { await workspace.loadMembers(channelID: readable.channel.id) } }
            .task(id: attachments?.syncRequest) { await attachments?.runAttachments() }
            .task(id: reactions?.syncRequest) { await reactions?.runReactions() }
            .task(id: readable?.reactionResyncTicket) {
                guard let readable, let ticket = readable.reactionResyncTicket else { return }
                do { try await Task.sleep(for: .seconds(15)) } catch { return }
                if readable.reactionResyncTicket == ticket { readable.retryAccess() }
            }
            .task(id: needsEmojiCatalog) {
                guard needsEmojiCatalog else { return }
                async let emojis: Void = workspace.loadEmojis()
                async let accounts: Void = workspace.loadEmojiAccounts()
                _ = await (emojis, accounts)
            }
            .task(id: authors) {
                if let readable { await workspace.resolveAuthors(in: readable.messages) }
            }
    }
}
