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
                TextField(parentID == nil ? "メッセージ" : "スレッドに返信", text: text, axis: .vertical)
                    .font(.body).lineLimit(1...6).padding(8)
                    .focused($focused).accessibilityIdentifier(parentID == nil ? "composer" : "replyComposer")
                Button {
                    let submitted = text.wrappedValue
                    Task { _ = await session.perform { try await conversation.send(submitted, parentID: parentID) } }
                } label: {
                    Group {
                        if sending { ProgressView().tint(.zooBrandForeground) }
                        else { Image(systemName: "paperplane").font(.system(size: 20, weight: .semibold)) }
                    }.foregroundStyle(Color.zooBrandForeground).frame(width: 44, height: 44)
                        .background(Color.zooAccent, in: RoundedRectangle(cornerRadius: 8))
                }.disabled(sending || text.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || text.wrappedValue.unicodeScalars.count > 40_000)
                    .opacity(text.wrappedValue.isEmpty ? 0.4 : 1)
                    .accessibilityLabel("送信").accessibilityIdentifier(parentID == nil ? "sendButton" : "sendReplyButton")
            }
            .padding(6)
            .background(Color.zooBackground, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.zooBorder, lineWidth: 1))
            .padding(10)
        }.background(Color.zooBackground)
        .toolbar {
            ToolbarItemGroup(placement: .keyboard) {
                Spacer()
                Button("完了") { focused = false }
                    .accessibilityIdentifier("dismissKeyboard")
            }
        }
    }
}
