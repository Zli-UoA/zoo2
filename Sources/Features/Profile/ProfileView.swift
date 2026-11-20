import SwiftUI

enum AppearancePreference: String {
    static let key = "zoo.appearance"
    case system, light, dark

    var colorScheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}

enum ProfileDestination: Hashable { case settings }

struct ProfileView: View {
    let workspace: WorkspaceStore
    let session: AppSession

    private var profile: PublicProfile? { workspace.publicProfile }
    private var loadError: String? { workspace.profileLoadError }

    private var user: Person { workspace.people[workspace.user.id] ?? workspace.user }
    private var server: URL? { try? ConnectionSettings.secureURL(workspace.client.serverURL) }

    var body: some View {
        AppForm {
            Section {
                HStack(spacing: 12) {
                    AvatarView(name: user.name, url: user.avatarURL, dimension: 56)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(user.name).font(.title2.bold())
                        Text("@\(user.user_name)").font(.subheadline).foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 4)
                if let url = workspace.client.accountProfileURL {
                    Link("名前・アイコンを編集", destination: url)
                }
            }
            Section("公開プロフィール") {
                if let profile {
                    LabeledContent("表示名", value: profile.name)
                    if !profile.summary.isEmpty { Text(profile.summary).textSelection(.enabled) }
                    Button("公開プロフィールを編集") { workspace.presentedSheet = .profile }
                        .accessibilityIdentifier("profile-edit")
                } else if loadError == nil {
                    ProgressView("読み込み中…")
                }
                if let loadError {
                    Text(loadError).foregroundStyle(.red)
                }
                if let server {
                    Link("公開ページを見る", destination: server.appending(path: "public/users").appending(component: user.id))
                }
            }
            Section {
                NavigationLink(value: ProfileDestination.settings) {
                    Label("設定", systemImage: "gearshape")
                }
                .accessibilityIdentifier("workspace-settings")
            }
        }
        .navigationTitle("プロフィール")
        .navigationBarTitleDisplayMode(.large)
        .navigationDestination(for: ProfileDestination.self) { _ in
            ProfileSettingsView(session: session, webSettings: server?.appending(path: "settings"))
                .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
        }
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        let request = UUID()
        workspace.profileLoadRequest = request
        workspace.profileLoadError = nil
        do {
            let value = try await workspace.client.publicProfile()
            guard workspace.profileLoadRequest == request, !Task.isCancelled else { return }
            workspace.publicProfile = value
        } catch {
            guard workspace.profileLoadRequest == request, !Task.isCancelled else { return }
            if case ZooError.expiredSession = error { workspace.report(error) }
            else { workspace.profileLoadError = "公開プロフィールを読み込めませんでした。" }
        }
    }
}

struct ProfileEditor: View {
    let workspace: WorkspaceStore
    let saved: (PublicProfile) -> Void
    @State private var draft: PublicProfile
    @State private var saving = false
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    init(profile: PublicProfile, workspace: WorkspaceStore, saved: @escaping (PublicProfile) -> Void) {
        self.workspace = workspace
        self.saved = saved
        _draft = State(initialValue: profile)
    }

    var body: some View {
        NavigationStack {
            AppForm {
                Section {
                    TextField("表示名", text: $draft.display_name, prompt: Text(draft.default_display_name))
                        .accessibilityIdentifier("profile-name")
                } header: { Text("公開する表示名") } footer: {
                    Text("100文字以内。空欄にするとチャットの表示名を使用します。")
                }
                Section {
                    TextField("自己紹介", text: $draft.summary, axis: .vertical)
                        .lineLimit(3...10).accessibilityIdentifier("profile-summary")
                } header: { Text("自己紹介") } footer: { Text("1,000文字以内。") }
                if let error { Text(error).foregroundStyle(.red) }
            }
            .disabled(saving)
            .navigationTitle("公開プロフィールを編集")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    AppCloseButton { dismiss() }.disabled(saving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("保存") { saving = true }
                        .disabled(!draft.canSave || saving)
                        .accessibilityIdentifier("profile-save")
                }
            }
            .interactiveDismissDisabled(saving)
            .task(id: saving) {
                guard saving else { return }
                do {
                    let value = try await workspace.client.updatePublicProfile(name: draft.display_name, summary: draft.summary)
                    try Task.checkCancellation()
                    saved(value)
                    dismiss()
                } catch {
                    guard !Task.isCancelled else { return }
                    if case ZooError.expiredSession = error { workspace.report(error) }
                    else { self.error = "保存できませんでした。もう一度お試しください。" }
                }
                saving = false
            }
        }
    }
}

private struct ProfileSettingsView: View {
    let session: AppSession
    let webSettings: URL?
    @State private var confirmingLogout = false
    @State private var enabling = false
    @AppStorage(AppearancePreference.key) private var appearance = AppearancePreference.system

    var body: some View {
        AppForm {
            Section("表示") {
                Picker("テーマ", selection: $appearance) {
                    Text("システムに合わせる").tag(AppearancePreference.system)
                    Text("ライト").tag(AppearancePreference.light)
                    Text("ダーク").tag(AppearancePreference.dark)
                }
            }
            Section("プッシュ通知") {
                Text(session.notifications.status)
                if session.notifications.enabled {
                    if session.notifications.canSendTest {
                        Button("テスト通知を送る") { Task { await session.notifications.sendTest() } }
                            .disabled(session.notifications.sendingTest)
                            .accessibilityIdentifier("send-test-push")
                    }
                    if let result = session.notifications.testStatus { Text(result) }
                    if session.notifications.registrationFailed {
                        Button("通知の登録を再試行") { enabling = true }.disabled(enabling)
                    }
                    Button("通知を無効にする") { session.notifications.disable() }
                } else {
                    Button("通知を有効にする") { enabling = true }.disabled(enabling)
                }
                if let url = URL(string: UIApplication.openNotificationSettingsURLString) {
                    Link("iOSの通知設定を開く", destination: url)
                }
            }
            SiriSettingsSection(workspace: session.workspace)
            if let webSettings {
                Section { Link("Web版の設定を開く", destination: webSettings) }
            }
            Section {
                Button("ログアウト", role: .destructive) { confirmingLogout = true }
                    .accessibilityIdentifier("logout")
            }
        }
        .navigationTitle("設定")
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog("ログアウトしますか？", isPresented: $confirmingLogout, titleVisibility: .visible) {
            Button("ログアウト", role: .destructive) { session.logout() }
            Button("キャンセル", role: .cancel) {}
        } message: { Text("この端末のログイン情報と下書きを削除します。") }
        .task(id: enabling) {
            if enabling { await session.notifications.enable(); enabling = false }
        }
    }
}
