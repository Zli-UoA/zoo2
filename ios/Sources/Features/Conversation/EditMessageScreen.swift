import SwiftUI

struct EditMessageScreen: View {
    @Environment(ConversationViewModel.self) private var conversation
    @Environment(SessionController.self) private var session
    @Environment(\.dismiss) private var dismiss
    let message: Message
    @State var text: String
    @State private var busy = false

    var body: some View {
        NavigationStack {
            TextEditor(text: $text).padding().accessibilityIdentifier("editMessage")
                .navigationTitle("メッセージを編集").navigationBarTitleDisplayMode(.inline)
                .interactiveDismissDisabled(busy)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("キャンセル") { dismiss() }.disabled(busy) }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("保存") {
                            busy = true
                            Task {
                                if await session.perform({ try await conversation.edit(message, content: text) }) { dismiss() }
                                busy = false
                            }
                        }.disabled(busy || text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || text.unicodeScalars.count > 40_000)
                    }
                }
        }
    }
}
