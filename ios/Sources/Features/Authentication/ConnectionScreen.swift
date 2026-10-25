import SwiftUI

struct ConnectionScreen: View {
    @Environment(SessionController.self) private var session
    @Environment(\.dismiss) private var dismiss
    @State var settings: ConnectionSettings

    var body: some View {
        NavigationStack {
            Form {
                Section("ワークスペース") { LabeledTextField(title: "サーバー URL", text: $settings.server, prompt: "https://zli.chat").keyboardType(.URL) }
                Section {
                    LabeledTextField(title: "Issuer URL", text: $settings.issuer).keyboardType(.URL)
                    LabeledTextField(title: "iOS 用 Client ID", text: $settings.clientID)
                    LabeledTextField(title: "Resource", text: $settings.resource).keyboardType(.URL)
                    LabeledTextField(title: "Scope", text: $settings.scope)
                } header: { Text("Zli アカウント") } footer: {
                    Text("管理者が発行した iOS 用の接続設定を入力してください。戻り先は https://zli.chat/ios/oauth/callback です。パスワードやクライアントシークレットは入力しません。")
                }
            }
            .textInputAutocapitalization(.never).autocorrectionDisabled()
            .navigationTitle("接続設定").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("キャンセル") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("保存") {
                        do { try session.saveSettings(settings); dismiss() }
                        catch { session.report(error) }
                    }
                }
            }
        }
    }
}
