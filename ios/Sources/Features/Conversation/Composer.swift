import SwiftUI

struct Composer: View {
    @Environment(ConversationViewModel.self) private var conversation
    @Environment(SessionController.self) private var session
    let channelID: String
    let parentID: String?
    private var sending: Bool { conversation.sending.contains(key) }
    @FocusState private var focused: Bool
    private var key: String { parentID ?? "root" }
    private var text: Binding<String> { Binding(get: { conversation.drafts[key] ?? "" }, set: { conversation.drafts[key] = $0 }) }

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(alignment: .bottom, spacing: 10) {
                TextField(parentID == nil ? "メッセージを送る" : "スレッドに返信", text: text, axis: .vertical)
                    .lineLimit(1...6).padding(12)
                    .background(Color(uiColor: .secondarySystemBackground), in: RoundedRectangle(cornerRadius: 20))
                    .focused($focused).accessibilityIdentifier(parentID == nil ? "composer" : "replyComposer")
                Button {
                    let submitted = text.wrappedValue
                    Task { _ = await session.perform { try await conversation.send(submitted, parentID: parentID) } }
                } label: {
                    Group {
                        if sending { ProgressView().tint(.white) }
                        else { Image(systemName: "arrow.up").font(.title3.bold()) }
                    }.foregroundStyle(.white).frame(width: 44, height: 44)
                        .background(Color.zooAccent, in: Circle())
                }.disabled(sending || text.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || text.wrappedValue.unicodeScalars.count > 40_000)
                    .opacity(text.wrappedValue.isEmpty ? 0.4 : 1)
                    .accessibilityLabel("送信").accessibilityIdentifier(parentID == nil ? "sendButton" : "sendReplyButton")
            }.padding(.horizontal, 14).padding(.vertical, 10)
        }.background(.bar)
    }
}
