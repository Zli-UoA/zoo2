import SwiftUI

struct ConnectionScreen: View {
    @Environment(SessionController.self) private var session
    @Environment(\.dismiss) private var dismiss
    @State var settings: ConnectionSettings

    var body: some View {
        NavigationStack {
            Form {
                Section("ワークスペース") { TextField("サーバー URL", text: $settings.server).keyboardType(.URL) }
                Section {
                    TextField("Issuer URL", text: $settings.issuer).keyboardType(.URL)
                    TextField("iOS 用 Client ID", text: $settings.clientID)
                    TextField("Resource", text: $settings.resource).keyboardType(.URL)
                    TextField("Scope", text: $settings.scope)
                } header: { Text("Zli アカウント") } footer: {
                    Text("管理者が発行した iOS 用の接続設定を入力してください。戻り先は zoo2://oauth/callback です。パスワードやクライアントシークレットは入力しません。")
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
