import SwiftUI

struct CreateChannelScreen: View {
    @Environment(ChannelViewModel.self) private var model
    @Environment(SessionController.self) private var session
    @Environment(\.dismiss) private var dismiss
    let created: (Channel) -> Void
    @State private var name = ""
    @State private var description = ""
    @State private var isPrivate = false
    @State private var busy = false

    var body: some View {
        NavigationStack {
            Form {
                Section("チャンネル") {
                    TextField("名前", text: $name).accessibilityIdentifier("channelName")
                    TextField("どんな話をする場所？", text: $description, axis: .vertical).lineLimit(2...4)
                }
                Section { Toggle("プライベート", isOn: $isPrivate) } footer: { Text("プライベートチャンネルは参加メンバーだけが閲覧できます。") }
            }
            .navigationTitle("新しいチャンネル").navigationBarTitleDisplayMode(.inline)
            .interactiveDismissDisabled(busy)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("キャンセル") { dismiss() }.disabled(busy) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("作成") {
                        busy = true
                        Task {
                            _ = await session.perform {
                                let channel = try await model.createChannel(name: name.trimmingCharacters(in: .whitespacesAndNewlines), description: description, isPrivate: isPrivate)
                                dismiss(); created(channel)
                            }
                            busy = false
                        }
                    }.disabled(busy || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || name.unicodeScalars.count > 80 || description.unicodeScalars.count > 250)
                        .accessibilityIdentifier("confirmCreate")
                }
            }
        }
    }
}
