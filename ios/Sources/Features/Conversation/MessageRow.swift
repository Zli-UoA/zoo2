import SwiftUI

struct MessageRow: View {
    @Environment(ChannelViewModel.self) private var channels
    @Environment(ConversationViewModel.self) private var conversation
    @Environment(SessionController.self) private var session
    let message: Message
    let canPost: Bool
    let showsThread: Bool
    let openThread: (Message) -> Void
    @State private var editing = false
    @State private var deleting = false
    @State private var busy = false
    private var author: String { channels.name(message.user_id) }
    private var emoji: [String] { Array(Set(conversation.reactions.filter { $0.message_id == message.id }.map(\.emoji))).sorted() }
    private var replies: Int { conversation.replyCounts[message.id, default: 0] }

    var body: some View {
        HStack(alignment: .top, spacing: 11) {
            Avatar(name: author)
            VStack(alignment: .leading, spacing: 7) {
                HStack(alignment: .firstTextBaseline, spacing: 7) {
                    Text(author).font(.subheadline.bold())
                    if channels.person(message.user_id)?.is_bot == true { Text("BOT").font(.caption2.monospaced()).foregroundStyle(.secondary) }
                    if let date = ZooDate.parse(message.created_at) { Text(date, style: .time).font(.caption2).foregroundStyle(.secondary) }
                    if message.updated_at != nil { Text("編集済み").font(.caption2).foregroundStyle(.secondary) }
                }
                if message.deleted_at != nil { Text("メッセージは削除されました").font(.callout).foregroundStyle(.secondary).italic() }
                else {
                    Text(message.content).font(.body).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityIdentifier("message-\(message.id)")
                    if let raw = message.card, let card = IntegrationCard.parse(raw) { IntegrationCardView(card: card) }
                    if !emoji.isEmpty {
                        ViewThatFits(in: .horizontal) {
                            reactionButtons
                            ScrollView(.horizontal, showsIndicators: false) { reactionButtons }
                        }
                    }
                    if showsThread && replies > 0 {
                        Button { openThread(message) } label: {
                            Label("\(replies) 件の返信", systemImage: "bubble.left.and.bubble.right").font(.caption.bold())
                        }.padding(.vertical, 5).accessibilityIdentifier("thread-\(message.id)")
                    }
                }
            }
        }
        .padding(.horizontal, 18).padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .contextMenu {
            if message.deleted_at == nil {
                Button("コピー", systemImage: "doc.on.doc") { UIPasteboard.general.string = message.content }
                if showsThread { Button("スレッドを開く", systemImage: "bubble.left") { openThread(message) } }
                if canPost {
                    Menu("リアクション", systemImage: "face.smiling") {
                        ForEach(["👍", "❤️", "🎉", "👀", "🙌", "☕️"], id: \.self) { emoji in
                            Button(emoji) { react(emoji) }.disabled(busy)
                        }
                    }
                    if message.user_id == channels.me?.id {
                        Button("編集", systemImage: "pencil") { editing = true }
                        Button("削除", systemImage: "trash", role: .destructive) { deleting = true }
                    }
                }
            }
        }
        .confirmationDialog("このメッセージを削除しますか？", isPresented: $deleting, titleVisibility: .visible) {
            Button("削除", role: .destructive) { Task { _ = await session.perform { try await conversation.delete(message) } } }
        }
        .sheet(isPresented: $editing) { EditMessageScreen(message: message, text: message.content) }
    }

    private var reactionButtons: some View {
        HStack(spacing: 6) {
            ForEach(emoji, id: \.self) { symbol in
                let matches = conversation.reactions.filter { $0.message_id == message.id && $0.emoji == symbol }
                let mine = matches.contains { $0.user_id == channels.me?.id }
                Button { react(symbol) } label: {
                    HStack(spacing: 4) { Text(symbol); Text("\(matches.count)").font(.caption.weight(.medium)) }
                        .padding(.horizontal, 10).frame(minHeight: 44)
                        .background(mine ? Color.zooAccent.opacity(0.12) : Color(uiColor: .secondarySystemBackground), in: Capsule())
                }.buttonStyle(.plain).disabled(!canPost || busy)
                    .accessibilityLabel("\(symbol) \(matches.count) 件\(mine ? "、リアクション済み" : "")")
            }
        }
    }

    private func react(_ emoji: String) {
        busy = true
        Task { _ = await session.perform { try await conversation.react(message, emoji: emoji) }; busy = false }
    }
}
