import SwiftUI

struct ActivityScreen: View {
    @Environment(ZooStore.self) private var store
    @State private var onlyUnread = false
    @State private var path: [ChannelRoute] = []

    var body: some View {
        NavigationStack(path: $path) {
            List {
                Toggle("未読のみ", isOn: $onlyUnread)
                let visible = store.activities.filter { !onlyUnread || !$0.is_read }
                if visible.isEmpty {
                    ContentUnavailableView("通知はありません", systemImage: "bell.badge", description: Text("メンションや返信がここに届きます。"))
                }
                ForEach(visible) { activity in
                    Button {
                        guard let channel = store.channel(activity.channel_id) else {
                            store.error = "この通知のチャンネルはまだ読み込まれていないか、閲覧できません。"; return
                        }
                        path.append(ChannelRoute(channel: channel, messageID: activity.message_id))
                        Task { _ = await store.perform { try await store.readActivity(activity) } }
                    } label: {
                        HStack(alignment: .top, spacing: 12) {
                            Avatar(name: store.name(activity.actor_id))
                            VStack(alignment: .leading, spacing: 5) {
                                Text(store.name(activity.actor_id)).font(.subheadline.bold())
                                Text(activity.summary).font(.subheadline)
                                if let channel = store.channel(activity.channel_id) {
                                    Text(store.title(channel)).font(.caption).foregroundStyle(.secondary)
                                }
                                if let date = ZooDate.parse(activity.created_at) { Text(date, style: .relative).font(.caption2).foregroundStyle(.secondary) }
                            }
                            Spacer()
                            if !activity.is_read { Circle().fill(Color.zooAccent).frame(width: 8, height: 8).accessibilityLabel("未読") }
                        }.padding(.vertical, 6).foregroundStyle(.primary)
                    }
                }
            }
            .navigationTitle("通知")
            .navigationDestination(for: ChannelRoute.self) { ChannelScreen(route: $0) }
            .refreshable { store.reconnect() }
        }
    }
}

struct ProfileScreen: View {
    @Environment(ZooStore.self) private var store
    @State private var displayName = ""
    @State private var busy = false
    @State private var confirmingLogout = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    HStack(spacing: 16) {
                        Avatar(name: store.me?.display_name ?? "", size: 64)
                        VStack(alignment: .leading, spacing: 5) {
                            Text(store.me?.display_name ?? "").font(.title2.bold())
                            Text("@\(store.me?.user_name ?? "")").foregroundStyle(.secondary)
                            Text("Zli メンバー").font(.caption).foregroundStyle(.secondary)
                        }
                    }.padding(.vertical, 10)
                }
                Section("プロフィール") {
                    TextField("表示名", text: $displayName)
                    Button("表示名を保存") {
                        busy = true
                        Task { _ = await store.perform { try await store.updateName(displayName.trimmingCharacters(in: .whitespacesAndNewlines)) }; busy = false }
                    }.disabled(busy || displayName == store.me?.display_name || displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || displayName.unicodeScalars.count > 80)
                }
                Section("アプリについて") {
                    LabeledContent("アプリ", value: "Zoo for iOS")
                    LabeledContent("ワークスペース", value: "Zli")
                    if store.isDemo { Text("デモの変更はログアウトすると消えます。").font(.caption).foregroundStyle(.secondary) }
                    else { Text("通知はアプリ内で確認できます。このバージョンはバックグラウンドのプッシュ通知に対応していません。").font(.caption).foregroundStyle(.secondary) }
                }
                Section {
                    Button(store.isDemo ? "デモを終了" : "ログアウト", role: .destructive) { confirmingLogout = true }
                        .accessibilityIdentifier("logoutButton")
                }
            }
            .navigationTitle("あなた")
            .onAppear { displayName = store.me?.display_name ?? "" }
            .confirmationDialog("ログアウトしますか？", isPresented: $confirmingLogout, titleVisibility: .visible) {
                Button("ログアウト", role: .destructive) { store.logout() }
            } message: { Text("この端末のログイン情報と下書きを削除します。") }
        }
    }
}
