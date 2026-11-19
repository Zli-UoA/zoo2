import SwiftUI

struct CreateChannelView: View {
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var description = ""
    @State private var isPrivate = false
    @State private var submitting = false
    @State private var error: String?
    @State private var id = UUID().uuidString.lowercased()

    var body: some View {
        NavigationStack {
            AppForm {
                Section("新しいチャンネル") {
                    TextField("チャンネル名", text: $name).textInputAutocapitalization(.never).autocorrectionDisabled()
                        .accessibilityIdentifier("channel-name")
                    TextField("説明（任意）", text: $description, axis: .vertical)
                    Toggle("非公開", isOn: $isPrivate)
                }
                if let error { Text(error).foregroundStyle(.red) }
            }
            .disabled(submitting)
            .navigationTitle("チャンネルを作成").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("作成") { submitting = true }
                        .disabled(submitting || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("create-channel")
                }
            }
            .task(id: submitting) {
                guard submitting else { return }
                defer { submitting = false }
                do {
                    let channel = try await workspace.client.createChannel(id: id, name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                                                                           description: description, isPrivate: isPrivate)
                    try Task.checkCancellation()
                    workspace.addAndOpen(channel)
                    dismiss()
                } catch {
                    if !Task.isCancelled { self.error = error.localizedDescription; workspace.report(error) }
                }
            }
        }
    }
}
