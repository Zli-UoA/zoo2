import SwiftUI

@MainActor
final class MessageActionSheet: Identifiable {
    let id = UUID()
    let store: ConversationStore
    let message: Message
    let parentID: String?
    let canOpenThread: Bool
    let wasComposing: Bool
    var replyRequested = false

    init(store: ConversationStore, message: Message, parentID: String?, canOpenThread: Bool,
         wasComposing: Bool) {
        self.store = store
        self.message = message
        self.parentID = parentID
        self.canOpenThread = canOpenThread
        self.wasComposing = wasComposing
    }

    func finish(in workspace: WorkspaceStore) {
        if replyRequested, parentID == nil {
            workspace.openThread(message.id, in: store, focusComposer: store.canPost)
        } else if (replyRequested || wasComposing), store.canPost {
            workspace.composerFocusRequest = ComposerFocusRequest(channelID: store.channel.id, parentID: parentID)
        }
    }
}

struct MessageActionsView: View {
    let store: ConversationStore
    let workspace: WorkspaceStore
    let message: Message
    let canOpenThread: Bool
    let reply: () -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @State private var page = Page.actions
    @State private var editingMessage: Message?
    @State private var confirmingDelete = false
    @ScaledMetric(relativeTo: .body) private var stampSize: CGFloat = 44

    private enum Page { case actions, reactions, edit }
    private var currentMessage: Message? { store.message(id: message.id) }
    private var canRetainEditor: Bool { store.active && !store.blocked && currentMessage?.user_id == store.userID }
    private var canEdit: Bool { store.canPost && canRetainEditor }
    private var detents: Set<PresentationDetent> {
        if dynamicTypeSize.isAccessibilitySize || page == .edit { return [.large] }
        if page == .reactions { return [.medium, .large] }
        let height = 56 + (store.canPost ? stampSize + 12 : 0) + (canOpenThread ? 96 : 0)
        return [.height(height + (currentMessage?.content.isEmpty == false ? 48 : 0) + (canEdit ? 98 : 0)), .large]
    }

    var body: some View {
        Group {
            switch page {
            case .actions: actions
            case .reactions: ReactionPicker(workspace: workspace, select: react)
            case .edit:
                if let editingMessage { EditMessageView(store: store, message: editingMessage) }
            }
        }
        .presentationDetents(detents)
        .presentationDragIndicator(.visible)
        .alert("メッセージを削除しますか？", isPresented: $confirmingDelete) {
            Button("削除", role: .destructive) {
                Task { if await store.delete(message) { dismiss() } }
            }
            .disabled(store.mutationInProgress || !canEdit)
            Button("キャンセル", role: .cancel) {}
        }
        .onChange(of: store.canReact(to: message.id)) { _, allowed in
            if page == .reactions && !allowed { page = .actions }
        }
        .onChange(of: canRetainEditor) { _, allowed in
            if !allowed {
                if page == .edit { page = .actions }
                confirmingDelete = false
            }
        }
        .onChange(of: store.accessGeneration) { _, _ in
            if page == .edit { page = .actions }
            confirmingDelete = false
        }
    }

    private func react(_ emoji: String) {
        guard store.canReact(to: message.id) else { return }
        dismiss()
        Task { await store.toggleReaction(messageID: message.id, emoji: emoji) }
    }

    private var actions: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if store.mutationInProgress { ProgressView("変更を保存しています…") }
                if let error = store.actionError { Text(error).foregroundStyle(.red) }
                if store.canPost {
                    stampBar
                        .disabled(!store.canReact(to: message.id))
                }

                if canOpenThread {
                    Button(action: reply) {
                        VStack(spacing: 6) {
                            Image(systemName: "bubble.left").font(.title2)
                            Text(store.canPost ? "返信する" : "スレッドを表示").font(.subheadline.bold())
                        }
                        .frame(maxWidth: .infinity, minHeight: 76)
                        .padding(.vertical, 4)
                        .background(AppDesign.surface, in: .rect(cornerRadius: 16))
                    }
                    .accessibilityIdentifier("message-action-reply")
                }

                VStack(spacing: 0) {
                    if currentMessage?.content.isEmpty == false {
                        Button {
                            if let current = store.message(id: message.id), !current.content.isEmpty {
                                UIPasteboard.general.string = current.content
                            }
                            dismiss()
                        } label: { actionLabel("メッセージをコピー", icon: "document.on.document") }
                            .accessibilityIdentifier("message-action-copy")
                    }
                    if canEdit {
                        Button {
                            guard let current = store.message(id: message.id) else { dismiss(); return }
                            editingMessage = current
                            page = .edit
                        } label: { actionLabel("メッセージを編集", icon: "pencil") }
                            .accessibilityIdentifier("message-action-edit")
                        Divider()
                        Button(role: .destructive) { confirmingDelete = true } label: { actionLabel("メッセージを削除", icon: "trash") }
                            .foregroundStyle(.red)
                            .disabled(store.mutationInProgress)
                            .accessibilityIdentifier("message-action-delete")
                    }
                }
            }
            .buttonStyle(.plain)
            .padding(.horizontal, AppDesign.horizontalInset)
            .padding(.top, 26)
            .padding(.bottom, 12)
        }
        .scrollBounceBehavior(.basedOnSize)
    }

    private var stampBar: some View {
        let groups = store.reactions(for: message.id)
        let choices = quickStamps(groups)
        let size = max(44, stampSize)
        return HStack(spacing: 6) {
            ScrollView(.horizontal) {
                HStack(spacing: 6) {
                    ForEach(choices, id: \.self) { emoji in
                        let glyph = ReactionGlyph(emoji: emoji, message: message, workspace: workspace)
                        let selected = groups.contains { $0.emoji == emoji && $0.isMine }
                        Button { react(emoji) } label: {
                            glyph
                                .padding(4)
                                .frame(width: size * 1.4, height: size)
                                .background(selected ? AppDesign.selectionBackground : AppDesign.surface, in: Capsule())
                                .overlay { if selected { Capsule().strokeBorder(AppDesign.navigationTint) } }
                                .contentShape(Capsule())
                        }
                        .disabled(store.isReactionPending(messageID: message.id, emoji: emoji))
                        .accessibilityLabel(glyph.label)
                        .accessibilityHint(selected ? "スタンプを取り消します" : "スタンプを付けます")
                        .accessibilityAddTraits(selected ? .isSelected : [])
                        .accessibilityIdentifier("message-action-quick-\(emoji)")
                    }
                }
            }
            .scrollIndicators(.hidden)
            .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
            Button { page = .reactions } label: {
                Image(systemName: "face.smiling.inverse")
                    .font(.title2)
                    .overlay(alignment: .topTrailing) {
                        Image(systemName: "plus.circle.fill")
                            .font(.caption.bold()).symbolRenderingMode(.palette)
                            .foregroundStyle(.primary, AppDesign.surface)
                            .offset(x: 4, y: -2)
                    }
                    .frame(width: size * 1.4, height: size)
                    .background(AppDesign.surface, in: Capsule())
                    .contentShape(Capsule())
            }
            .accessibilityLabel("スタンプを追加")
            .accessibilityIdentifier("message-action-more-reactions")
        }
    }

    private func quickStamps(_ groups: [ReactionGroup]) -> [String] {
        let registered = [("草", "kusa"), ("わかる", "wakaru"), ("はい", "hai"),
                          ("いいえ", "iie"), ("かなしい", "kanashii"), ("なるほど", "naruhodo")]
            .compactMap { name, alias in
                (workspace.emoji(for: ":\(name):") ?? workspace.emoji(for: ":\(alias):")).map { ":\($0.name):" }
            }
        let existing = groups.filter { !$0.emoji.hasPrefix("apub:") }.map(\.emoji)
        return workspace.recentEmojis.choices(fallback: existing + registered + ["👍", "❤️", "🎉", "😊", "😢", "👏"],
                                            canonical: store.canonicalEmoji)
    }

    private func actionLabel(_ title: LocalizedStringKey, icon: String) -> some View {
        HStack(spacing: AppDesign.spacing) {
            Image(systemName: icon).font(.body).frame(width: 24)
            Text(title)
        }
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
        .contentShape(Rectangle())
    }
}
