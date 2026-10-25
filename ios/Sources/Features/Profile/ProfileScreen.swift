import SwiftUI

struct ProfileScreen: View {
    @Environment(ProfileViewModel.self) private var model
    @Environment(SessionController.self) private var session
    @State private var displayName = ""
    @State private var busy = false
    @State private var confirmingLogout = false
    @State private var saved = false
    private var trimmedName: String { displayName.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    HStack(spacing: 16) {
                        Avatar(name: model.me?.display_name ?? "", size: 64)
                        VStack(alignment: .leading, spacing: 5) {
                            Text(model.me?.display_name ?? "").font(.title3.bold())
                            Text("@\(model.me?.user_name ?? "")").foregroundStyle(.secondary)
                            Text("Zli メンバー").font(.caption).foregroundStyle(.secondary)
                        }
                    }.padding(.vertical, 10)
                }
                Section("プロフィール") {
                    LabeledTextField(title: "表示名", text: $displayName, identifier: "displayName")
                        .disabled(busy)
                        .onChange(of: displayName) { _, _ in saved = false }
                    Button(busy ? "保存中…" : "表示名を保存") {
                        busy = true
                        Task { saved = await session.perform { try await model.updateName(trimmedName) }; busy = false }
                    }.disabled(busy || trimmedName == model.me?.display_name || trimmedName.isEmpty || displayName.unicodeScalars.count > 80)
                    if saved { Label("保存しました", systemImage: "checkmark.circle").font(.caption).foregroundStyle(Color.zooAccent) }
                }
                if !session.isDemo {
                    Section("プッシュ通知") {
                        Text(session.notifications.status).font(.caption).foregroundStyle(.secondary)
                        Button(session.notifications.enabled ? "通知を再登録" : "通知を有効にする") {
                            Task { await session.notifications.enable() }
                        }
                        if session.notifications.enabled {
                            Button("通知を無効にする", role: .destructive) { session.notifications.disable() }
                        }
                    }
                }
                Section("アプリについて") {
                    LabeledContent("アプリ", value: "Zoo for iOS")
                    LabeledContent("ワークスペース", value: "Zli")
                    if session.isDemo { Text("デモの変更はログアウトすると消えます。").font(.caption).foregroundStyle(.secondary) }

                }
                Section {
                    Button(session.isDemo ? "デモを終了" : "ログアウト", role: .destructive) { confirmingLogout = true }
                        .accessibilityIdentifier("logoutButton")
                }
            }
            .navigationTitle("あなた").navigationBarTitleDisplayMode(.inline)
            .onAppear { displayName = model.me?.display_name ?? "" }
            .confirmationDialog("ログアウトしますか？", isPresented: $confirmingLogout, titleVisibility: .visible) {
                Button("ログアウト", role: .destructive) { session.logout() }
            } message: { Text("この端末のログイン情報と下書きを削除します。") }
        }
    }
}
