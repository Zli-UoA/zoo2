import SwiftUI

struct CreateChannelView: View {
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var description = ""
    @State private var isPrivate = false
    @State private var encrypted = false
    @State private var submitting = false
    @State private var error: String?
    @State private var id = UUID().uuidString.lowercased()

    var body: some View {
        NavigationStack {
            AppForm {
                Section {
                    TextField("チャンネル名", text: $name).textInputAutocapitalization(.never).autocorrectionDisabled()
                        .accessibilityIdentifier("channel-name")
                    TextField("説明（任意）", text: $description, axis: .vertical)
                    Toggle("非公開", isOn: $isPrivate)
                    // 暗号化は、非公開のチャンネルだけ（メッセージは、メンバーの端末だけが読める）。
                    if workspace.e2ee != nil, isPrivate {
                        Toggle("暗号化", isOn: $encrypted).accessibilityIdentifier("channel-encrypted")
                    }
                } header: {
                    Text("新しいチャンネル")
                } footer: {
                    if encrypted && isPrivate { Text("暗号化したチャンネルは、あとから暗号化をやめられません。サーバーにも、運営にも本文は読めません。") }
                }
                if let error { Text(error).foregroundStyle(.red) }
            }
            .disabled(submitting)
            .onChange(of: isPrivate) { _, value in if !value { encrypted = false } }
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
                                                                           description: description, isPrivate: isPrivate, encrypted: isPrivate && encrypted)
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
