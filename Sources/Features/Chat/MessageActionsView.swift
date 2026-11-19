import SwiftUI

struct MessageActionsView: View {
    let store: ConversationStore
    let workspace: WorkspaceStore
    let message: Message
    let canOpenThread: Bool
    let reply: () -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @State private var page = Page.actions
    @State private var confirmingDelete = false
    @ScaledMetric(relativeTo: .body) private var stampSize: CGFloat = 44

    private enum Page { case actions, reactions, edit }
    private var canEdit: Bool { store.canPost && message.user_id == store.userID }
    private var detents: Set<PresentationDetent> {
        if dynamicTypeSize.isAccessibilitySize || page == .edit { return [.large] }
        if page == .reactions { return [.medium, .large] }
        let height = 56 + (store.canPost ? stampSize + 12 : 0) + (canOpenThread ? 96 : 0)
        return [.height(height + (message.content.isEmpty ? 0 : 48) + (canEdit ? 98 : 0)), .large]
    }

    var body: some View {
        Group {
            switch page {
            case .actions: actions
            case .reactions: ReactionPicker(workspace: workspace, select: react)
            case .edit: EditMessageView(store: store, message: message)
            }
        }
        .presentationDetents(detents)
        .presentationDragIndicator(.visible)
        .alert("メッセージを削除しますか？", isPresented: $confirmingDelete) {
            Button("削除", role: .destructive) {
                dismiss()
                Task { await store.delete(message) }
            }
            Button("キャンセル", role: .cancel) {}
        }
        .onChange(of: store.canReact(to: message.id)) { _, allowed in
            if page == .reactions && !allowed { page = .actions }
        }
        .onChange(of: canEdit) { _, allowed in
            if !allowed {
                if page == .edit { page = .actions }
                confirmingDelete = false
            }
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
                        .background(Color(.secondarySystemBackground), in: .rect(cornerRadius: 16))
                    }
                    .accessibilityIdentifier("message-action-reply")
                }

                VStack(spacing: 0) {
                    if !message.content.isEmpty {
                        Button {
                            UIPasteboard.general.string = message.content
                            dismiss()
                        } label: { actionLabel("メッセージをコピー", icon: "document.on.document") }
                            .accessibilityIdentifier("message-action-copy")
                    }
                    if canEdit {
                        Button { page = .edit } label: { actionLabel("メッセージを編集", icon: "pencil") }
                            .accessibilityIdentifier("message-action-edit")
                        Divider()
                        Button(role: .destructive) { confirmingDelete = true } label: { actionLabel("メッセージを削除", icon: "trash") }
                            .foregroundStyle(.red)
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
                                .background(selected ? AppDesign.tint.opacity(0.14) : Color(.secondarySystemBackground), in: Capsule())
                                .overlay { if selected { Capsule().strokeBorder(.tint) } }
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
                            .foregroundStyle(.primary, Color(.secondarySystemBackground))
                            .offset(x: 4, y: -2)
                    }
                    .frame(width: size * 1.4, height: size)
                    .background(Color(.secondarySystemBackground), in: Capsule())
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
