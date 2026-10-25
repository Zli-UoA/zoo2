import SwiftUI

struct MessageTimeline: View {
    @Environment(ConversationViewModel.self) private var conversation
    @Environment(SessionController.self) private var session
    let channelID: String
    let parentID: String?
    let targetID: String?
    let canPost: Bool
    let openThread: (Message) -> Void
    @State private var limit = 100
    @State private var atBottom = true
    @State private var positioned = false
    @State private var markingRead = false

    private var rows: [Message] {
        conversation.timeline.filter { $0.channel_id == channelID && ($0.parent_id == parentID || $0.id == parentID) }
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if rows.count > limit {
                        Button("以前のメッセージを表示") { limit += 100 }.frame(maxWidth: .infinity).padding()
                    }
                    if conversation.loadingMessages && rows.isEmpty {
                        ProgressView("メッセージを読み込み中").frame(maxWidth: .infinity).padding(40)
                    } else if rows.isEmpty {
                        ContentUnavailableView(parentID == nil ? "ここから会話をはじめよう" : "最初の返信を送ろう", systemImage: "bubble.left", description: Text("アイデアも、ちょっとした一言も。"))
                    }
                    ForEach(Array(rows.suffix(limit))) { message in
                        MessageRow(message: message, canPost: canPost, showsThread: parentID == nil, openThread: openThread)
                            .id(message.id)
                    }
                    Color.clear.frame(height: 1).id("bottom")
                        .onAppear { atBottom = true }.onDisappear { atBottom = false }
                    if parentID == nil, let last = rows.last(where: { $0.deleted_at == nil }) {
                        Button("ここまで既読にする") {
                            markingRead = true
                            Task { _ = await session.perform { try await conversation.markRead(messageID: last.id) }; markingRead = false }
                        }.font(.caption).padding().frame(maxWidth: .infinity).disabled(markingRead)
                    }
                }.padding(.vertical, 10)
            }
            .scrollDismissesKeyboard(.interactively)
            .defaultScrollAnchor(.bottom)
            .onChange(of: rows, initial: true) { _, new in
                guard !new.isEmpty else { return }
                if !positioned {
                    if let targetID, let index = new.firstIndex(where: { $0.id == targetID }) {
                        limit = max(limit, new.count - index)
                        Task { @MainActor in
                            await Task.yield()
                            proxy.scrollTo(targetID, anchor: .center)
                        }
                    } else { proxy.scrollTo("bottom", anchor: .bottom) }
                    positioned = true
                } else if atBottom { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
            }
            .overlay(alignment: .bottomTrailing) {
                if !atBottom && !rows.isEmpty {
                    Button("最新へ", systemImage: "arrow.down") {
                        withAnimation { proxy.scrollTo("bottom", anchor: .bottom) }
                    }.font(.caption.bold()).buttonStyle(.borderedProminent).clipShape(Capsule()).padding()
                }
            }
        }
    }
}
