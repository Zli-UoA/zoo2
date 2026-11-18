import SwiftUI

struct WorkspaceView: View {
    @Bindable var workspace: WorkspaceStore
    let session: AppSession
    @Environment(\.scenePhase) private var scenePhase
    @State private var search = ""
    @State private var unreadOnly = false
    @State private var showingSettings = false
    @State private var showingCreate = false
    @State private var showingPeople = false

    private var visible: [Channel] {
        workspace.channels.filter {
            $0.archive_at == nil && (!unreadOnly || workspace.unread.contains($0.id)) &&
            (search.isEmpty || workspace.name(for: $0).localizedStandardContains(search) ||
             ($0.description ?? "").localizedStandardContains(search))
        }
    }

    var body: some View {
        NavigationStack(path: $workspace.path) {
            List {
                Section {
                    HStack(spacing: 12) {
                        AvatarView(name: workspace.user.name, url: workspace.people[workspace.user.id]?.avatarURL)
                        VStack(alignment: .leading, spacing: 3) {
                            Text("いつもの会話を、ここから。 ").font(.headline)
                            Text(workspace.user.name).font(.subheadline).foregroundStyle(.secondary)
                        }
                    }.padding(.vertical, 8)
                }
                if let error = workspace.error {
                    Section {
                        Label(error, systemImage: "wifi.exclamationmark")
                            .font(.callout).foregroundStyle(.secondary)
                        Button("再読み込み") { Task { await workspace.refresh() } }
                    }
                }
                if workspace.loading && !workspace.hasLoaded {
                    ProgressView("チャンネルを読み込んでいます…")
                } else if visible.isEmpty {
                    ContentUnavailableView(search.isEmpty ? "チャンネルがありません" : "見つかりませんでした",
                                           systemImage: search.isEmpty ? "bubble.left.and.bubble.right" : "magnifyingglass",
                                           description: Text(unreadOnly ? "未読の会話はありません。" : "検索条件を変えるか、新しい会話を始めてください。"))
                }
                channelSection("チャンネル", channels: visible.filter { !$0.is_dm })
                channelSection("ダイレクトメッセージ", channels: visible.filter(\.is_dm))
            }
            .navigationTitle("Zoo")
            .searchable(text: $search, prompt: "チャンネルを検索")
            .refreshable { await workspace.refresh() }
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("設定", systemImage: "person.crop.circle") { showingSettings = true }
                        .accessibilityIdentifier("workspace-settings")
                }
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button("未読のみ", systemImage: unreadOnly ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle") {
                        unreadOnly.toggle()
                    }.accessibilityValue(unreadOnly ? "有効" : "無効")
                    Menu("会話を始める", systemImage: "square.and.pencil") {
                        Button("チャンネルを作成", systemImage: "number") { showingCreate = true }
                        Button("ダイレクトメッセージ", systemImage: "person.2") { showingPeople = true }
                    }.accessibilityIdentifier("new-conversation")
                }
            }
            .navigationDestination(for: ChatRoute.self) { route in
                if let channel = workspace.channels.first(where: { $0.id == route.channelID }) {
                    ConversationView(store: workspace.conversation(for: channel), workspace: workspace, route: route)
                } else {
                    ContentUnavailableView("チャンネルを表示できません", systemImage: "bubble.left.and.exclamationmark.bubble.right")
                }
            }
            .sheet(isPresented: $showingSettings) { SettingsView(session: session, user: workspace.people[workspace.user.id] ?? workspace.user) }
            .sheet(isPresented: $showingCreate) { CreateChannelView(workspace: workspace) }
            .sheet(isPresented: $showingPeople) { PeopleView(workspace: workspace) }
        }
        .modifier(ConversationSync(workspace: workspace))
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.run() }
        }
    }

    @ViewBuilder
    private func channelSection(_ title: String, channels: [Channel]) -> some View {
        if !channels.isEmpty {
            Section(title) {
                ForEach(channels) { channel in
                    NavigationLink(value: ChatRoute.channel(channel.id)) {
                        HStack(spacing: 12) {
                            if let peer = workspace.directMessagePeer(for: channel) {
                                AvatarView(name: peer.name, url: peer.avatarURL)
                            } else {
                                Image(systemName: channel.symbol)
                                    .font(.title3).foregroundStyle(.tint)
                                    .frame(width: 32, height: 38)
                            }
                            VStack(alignment: .leading, spacing: 4) {
                                Text(workspace.name(for: channel))
                                    .fontWeight(workspace.unread.contains(channel.id) ? .bold : .medium)
                                if let description = channel.description, !description.isEmpty {
                                    Text(description).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                }
                            }
                            Spacer(minLength: 4)
                            if workspace.unread.contains(channel.id) {
                                Circle().fill(.tint).frame(width: 8, height: 8).accessibilityLabel("未読")
                            }
                        }.padding(.vertical, 3)
                    }
                    .accessibilityIdentifier("channel-\(channel.id)")
                }
            }
        }
    }
}

private struct ConversationSync: ViewModifier {
    let workspace: WorkspaceStore
    @Environment(\.scenePhase) private var scenePhase

    private struct Authors: Equatable {
        let channelID: String
        let ids: Set<String>
    }

    private var activeConversation: ConversationStore? {
        guard scenePhase == .active, let id = workspace.path.last?.channelID,
              let channel = workspace.channels.first(where: { $0.id == id }) else { return nil }
        return workspace.conversation(for: channel)
    }

    func body(content: Content) -> some View {
        let store = activeConversation
        let readable = store.flatMap { !$0.blocked && $0.channel.encrypted != true ? $0 : nil }
        let reactions = readable.flatMap { $0.needsReactionStream ? $0 : nil }
        let authors = readable.map { Authors(channelID: $0.channel.id, ids: Set($0.messages.map(\.user_id))) }
        content
            .task(id: readable?.syncRequest) { await readable?.run() }
            .task(id: reactions?.syncRequest) { await reactions?.runReactions() }
            .task(id: readable?.reactionResyncTicket) {
                guard let readable, let ticket = readable.reactionResyncTicket else { return }
                do { try await Task.sleep(for: .seconds(15)) } catch { return }
                if readable.reactionResyncTicket == ticket { readable.retryAccess() }
            }
            .task(id: store?.channel.id) {
                guard store != nil else { return }
                async let emojis: Void = workspace.loadEmojis()
                async let accounts: Void = workspace.loadEmojiAccounts()
                _ = await (emojis, accounts)
            }
            .task(id: authors) {
                if let readable { await workspace.resolveAuthors(in: readable.messages) }
            }
    }
}

private struct SettingsView: View {
    let session: AppSession
    let user: Person
    @Environment(\.dismiss) private var dismiss
    @State private var confirmingLogout = false
    @State private var enabling = false

    var body: some View {
        NavigationStack {
            Form {
                Section("アカウント") {
                    HStack(spacing: 12) {
                        AvatarView(name: user.name, url: user.avatarURL)
                        Text(user.name).font(.headline)
                    }
                    LabeledContent("表示名", value: user.name)
                    LabeledContent("ユーザー名", value: "@" + user.user_name)
                }
                Section("通知") {
                    Text(session.notifications.status).font(.callout)
                    if session.notifications.enabled {
                        Button("通知を無効にする") { session.notifications.disable() }
                    } else {
                        Button("通知を有効にする") { enabling = true }.disabled(enabling)
                    }
                    if let url = URL(string: UIApplication.openSettingsURLString) { Link("iOS の通知設定を開く", destination: url) }
                }
                Section {
                    Button("ログアウト", role: .destructive) { confirmingLogout = true }
                        .accessibilityIdentifier("logout")
                }
            }
            .navigationTitle("設定").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("完了") { dismiss() } } }
            .confirmationDialog("ログアウトしますか？", isPresented: $confirmingLogout, titleVisibility: .visible) {
                Button("ログアウト", role: .destructive) { dismiss(); session.logout() }
            } message: { Text("この端末のログイン情報と下書きを削除します。") }
            .task(id: enabling) {
                if enabling { await session.notifications.enable(); enabling = false }
            }
        }
    }
}

private struct CreateChannelView: View {
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
            Form {
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
                ToolbarItem(placement: .cancellationAction) { Button("キャンセル") { dismiss() } }
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

private struct PeopleView: View {
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""
    @State private var selected: String?
    @State private var error: String?

    var body: some View {
        NavigationStack {
            List {
                if let error { Text(error).foregroundStyle(.red) }
                ForEach(workspace.people.values.filter {
                    $0.id != workspace.user.id && !$0.is_bot &&
                    (search.isEmpty || $0.name.localizedStandardContains(search) || $0.user_name.localizedStandardContains(search))
                }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }) { person in
                    Button { selected = person.id } label: {
                        HStack {
                            AvatarView(name: person.name, url: person.avatarURL)
                            VStack(alignment: .leading) {
                                Text(person.name).foregroundStyle(.primary)
                                Text("@" + person.user_name).font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            if selected == person.id { ProgressView() }
                        }
                    }.disabled(selected != nil)
                }
            }
            .searchable(text: $search, prompt: "名前を検索")
            .navigationTitle("ダイレクトメッセージ").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() } } }
            .task(id: selected) {
                guard let selected else { return }
                defer { self.selected = nil }
                do {
                    let channel = try await workspace.client.directMessage(userID: selected)
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
