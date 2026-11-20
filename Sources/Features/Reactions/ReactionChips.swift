import SwiftUI

struct ReactionChips: View {
    let store: ConversationStore
    let workspace: WorkspaceStore
    let message: Message
    @State private var selectedEmoji: String?

    var body: some View {
        let groups = store.reactions(for: message.id)
        if !groups.isEmpty {
            ReactionFlowLayout {
                ForEach(groups, id: \.emoji) { group in
                    let glyph = ReactionGlyph(emoji: group.emoji, message: message, workspace: workspace)
                    let selected = group.isMine
                    HStack(spacing: 4) {
                        glyph
                        Text(group.count, format: .number).font(.caption.bold().monospacedDigit())
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 8).padding(.vertical, 4)
                    .foregroundStyle(selected ? AppDesign.selectionForeground : .primary)
                    .background(selected ? AppDesign.selectionBackground : AppDesign.surface, in: Capsule())
                    .overlay { Capsule().strokeBorder(selected ? AppDesign.navigationTint : Color(.separator), lineWidth: 1) }
                    .frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
                    .highPriorityGesture(LongPressGesture(minimumDuration: 0.5).exclusively(before: TapGesture()).onEnded { value in
                        switch value {
                        case .first: selectedEmoji = group.emoji
                        case .second: toggle(group.emoji)
                        }
                    })
                    .accessibilityElement(children: .ignore)
                    .accessibilityAddTraits(.isButton)
                    .accessibilityAction { toggle(group.emoji) }
                    .accessibilityAction(named: "リアクションした人を表示") { selectedEmoji = group.emoji }
                    .accessibilityLabel(glyph.label)
                    .accessibilityValue(selected ? "\(group.count)人、自分も付けています" : "\(group.count)人")
                    .accessibilityAddTraits(selected ? .isSelected : [])
                    .accessibilityIdentifier("reaction-\(message.id)-\(group.emoji)")
                }
            }
            .sheet(isPresented: Binding(get: { selectedEmoji != nil }, set: { if !$0 { selectedEmoji = nil } })) {
                if let selectedEmoji { ReactionPeopleView(store: store, workspace: workspace, message: message, emoji: selectedEmoji) }
            }
        }
    }

    private func toggle(_ emoji: String) {
        guard store.canReact(to: message.id), !emoji.hasPrefix("apub:"), !store.isReactionPending(messageID: message.id, emoji: emoji) else { return }
        Task { await store.toggleReaction(messageID: message.id, emoji: emoji) }
    }
}

struct ReactionFlowLayout: Layout {
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let frames = frames(subviews: subviews, width: proposal.width)
        return CGSize(width: frames.map(\.maxX).max() ?? 0, height: frames.map(\.maxY).max() ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let frames = frames(subviews: subviews, width: bounds.width)
        for (subview, frame) in zip(subviews, frames) {
            subview.place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
                          anchor: .topLeading, proposal: ProposedViewSize(frame.size))
        }
    }

    private func frames(subviews: Subviews, width: CGFloat?) -> [CGRect] {
        let sizes = subviews.map { subview in
            let ideal = subview.sizeThatFits(.unspecified)
            guard let width, width.isFinite, ideal.width > width else { return ideal }
            return subview.sizeThatFits(ProposedViewSize(width: max(0, width), height: nil))
        }
        return Self.frames(for: sizes, maxWidth: width ?? .infinity)
    }

    static func frames(for sizes: [CGSize], maxWidth: CGFloat) -> [CGRect] {
        let spacing: CGFloat = 5
        let size = CGSize(width: sizes.map(\.width).max() ?? 0, height: sizes.map(\.height).max() ?? 0)
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        return sizes.map { _ in
            if x > 0, x + size.width > maxWidth {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            let frame = CGRect(x: x, y: y, width: size.width, height: size.height)
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
            return frame
        }
    }
}

private struct ReactionPeopleView: View {
    let store: ConversationStore
    let workspace: WorkspaceStore
    let message: Message
    let emoji: String
    @State private var resolved: [String: Person] = [:]
    @State private var error: String?

    private var ids: [String] { store.reactionUserIDs(messageID: message.id, emoji: emoji) }

    var body: some View {
        AppList {
            HStack {
                ReactionGlyph(emoji: emoji, message: message, workspace: workspace)
                Text("\(ids.count)人がリアクションしました")
            }
            ForEach(ids, id: \.self) { id in
                if let person = workspace.people[id] ?? resolved[id] { PersonRow(person: person) }
                else { Label("ユーザー", systemImage: "person.crop.circle") }
            }
            if let error { Text(error).font(.caption).foregroundStyle(.secondary) }
        }
        .accessibilityIdentifier("reaction-people-sheet")
        .task(id: ids) {
            for id in ids where workspace.people[id] == nil && resolved[id] == nil {
                do {
                    let person = try await workspace.client.person(id: id)
                    try Task.checkCancellation()
                    resolved[id] = person
                } catch {
                    guard !Task.isCancelled else { return }
                    self.error = "一部のユーザー情報を取得できませんでした。"
                    if case ZooError.expiredSession = error { workspace.report(error) }
                }
            }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }
}
