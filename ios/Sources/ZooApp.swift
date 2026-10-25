import SwiftUI

@main
struct ZooApp: App {
    @State private var store = ZooStore()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(store)
                .tint(.zooAccent)
                .task {
                    if ProcessInfo.processInfo.arguments.contains("--demo") { store.loadDemo() }
                    else if !ProcessInfo.processInfo.arguments.contains("--uitesting") { await store.connect(interactive: false) }
                }
                .onChange(of: scenePhase) { _, phase in
                    if phase == .background { store.setActive(false) }
                    if phase == .active { store.setActive(true) }
                }
        }
    }
}

extension Color {
    static let zooAccent = Color(red: 0.73, green: 0.28, blue: 0.08)
}

struct RootView: View {
    @Environment(ZooStore.self) private var store

    var body: some View {
        @Bindable var store = store
        Group {
            if store.me != nil {
                VStack(spacing: 0) {
                    VStack(spacing: 0) {
                        if store.isDemo {
                            Text("デモ · この端末だけのサンプルデータ")
                                .font(.caption).frame(maxWidth: .infinity).padding(6)
                                .background(Color.orange.opacity(0.12))
                        }
                        if !store.syncErrors.isEmpty {
                            HStack {
                                Image(systemName: "wifi.exclamationmark")
                                Text("同期が停止しています").font(.caption)
                                Spacer()
                                Button("再接続") { store.reconnect() }.font(.caption.bold())
                            }.padding(10).background(Color.orange.opacity(0.15))
                            .accessibilityElement(children: .contain)
                        }
                    }
                    TabView {
                        ChannelListScreen(directMessages: false)
                            .tabItem { Label("ホーム", systemImage: "square.grid.2x2") }
                        ChannelListScreen(directMessages: true)
                            .tabItem { Label("DM", systemImage: "bubble.left.and.bubble.right") }
                        ActivityScreen()
                            .tabItem { Label("通知", systemImage: "bell") }.badge(store.pendingCount)
                        ProfileScreen()
                            .tabItem { Label("あなた", systemImage: "person.crop.circle") }
                    }
                }
            } else { WelcomeScreen() }
        }
        .alert("確認してください", isPresented: Binding(get: { store.error != nil }, set: { if !$0 { store.error = nil } })) {
            Button("閉じる", role: .cancel) { store.error = nil }
        } message: { Text(store.error ?? "") }
    }
}

struct ZooMark: View {
    var size: CGFloat = 64
    var body: some View {
        Text("z.").font(.system(size: size * 0.65, weight: .black, design: .rounded))
            .foregroundStyle(.white).frame(width: size, height: size)
            .background(Color.zooAccent.gradient, in: RoundedRectangle(cornerRadius: size * 0.28))
            .accessibilityLabel("Zoo")
    }
}

struct Avatar: View {
    let name: String
    var size: CGFloat = 40
    private var hue: Double { Double(name.unicodeScalars.reduce(0) { $0 + Int($1.value) } % 360) / 360 }
    var body: some View {
        Text(String(name.prefix(1)).uppercased()).font(.system(size: size * 0.42, weight: .semibold, design: .rounded))
            .foregroundStyle(Color(hue: hue, saturation: 0.65, brightness: 0.38))
            .frame(width: size, height: size)
            .background(Color(hue: hue, saturation: 0.16, brightness: 0.96), in: RoundedRectangle(cornerRadius: size * 0.32))
            .accessibilityHidden(true)
    }
}

struct WelcomeScreen: View {
    @Environment(ZooStore.self) private var store
    @State private var showSettings = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 28) {
                    ZooMark(size: 84).padding(.top, 55)
                    VStack(alignment: .leading, spacing: 12) {
                        Text("つくる。話す。\nここに集まる。").font(.system(.largeTitle, design: .rounded, weight: .bold))
                        Text("Zli のためのコミュニケーション。\n日々の雑談から、次のアイデアまで。")
                            .font(.body).foregroundStyle(.secondary).lineSpacing(5)
                    }
                    VStack(alignment: .leading, spacing: 20) {
                        Label("話題ごとのチャンネル", systemImage: "number")
                        Label("気軽につながる DM", systemImage: "bubble.left.and.bubble.right")
                        Label("会話をつなぐスレッド", systemImage: "arrowshape.turn.up.left")
                    }.font(.subheadline).padding(22).frame(maxWidth: .infinity, alignment: .leading)
                        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 24))
                    VStack(spacing: 14) {
                        Button {
                            if store.settings.clientID.isEmpty { showSettings = true }
                            else { Task { await store.connect(interactive: true) } }
                        } label: {
                            HStack {
                                if store.isConnecting { ProgressView().tint(.white) }
                                Text("Zli アカウントでログイン").fontWeight(.semibold)
                                Image(systemName: "arrow.right")
                            }.frame(maxWidth: .infinity).padding(.vertical, 10)
                        }.buttonStyle(.borderedProminent).controlSize(.large).disabled(store.isConnecting)
                        Button("デモを試す") { store.loadDemo() }.accessibilityIdentifier("demoButton")
                            .disabled(store.isConnecting)
                        Text("デモはサーバーに接続しません。")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(28).frame(maxWidth: 540)
            }
            .frame(maxWidth: .infinity)
            .background(Color(uiColor: .systemGroupedBackground))
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("接続設定", systemImage: "gearshape") { showSettings = true } } }
            .sheet(isPresented: $showSettings) { ConnectionScreen(settings: store.settings) }
        }
    }
}

struct ConnectionScreen: View {
    @Environment(ZooStore.self) private var store
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
                        do { try store.saveSettings(settings); dismiss() }
                        catch { store.report(error) }
                    }
                }
            }
        }
    }
}

struct ChannelRoute: Hashable {
    let channel: Channel
    var messageID: String? = nil
}

struct ChannelListScreen: View {
    @Environment(ZooStore.self) private var store
    let directMessages: Bool
    @State private var search = ""
    @State private var path: [ChannelRoute] = []
    @State private var creating = false
    @State private var includeArchived = false

    private var filtered: [Channel] {
        store.channels.filter { $0.is_dm == directMessages && $0.deleted_at == nil && (includeArchived || $0.archive_at == nil) &&
            (search.isEmpty || store.title($0).localizedCaseInsensitiveContains(search)) }
            .sorted { store.title($0).localizedStandardCompare(store.title($1)) == .orderedAscending }
    }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if !directMessages {
                    Section {
                        HStack(spacing: 14) {
                            ZooMark(size: 48)
                            VStack(alignment: .leading, spacing: 3) {
                                Text("Zli").font(.title2.bold())
                                Text("みんなのアイデアが集まる場所").font(.caption).foregroundStyle(.secondary)
                            }
                        }.padding(.vertical, 8)
                    }
                    Toggle("アーカイブ済みも表示", isOn: $includeArchived).font(.subheadline)
                }
                if filtered.isEmpty {
                    ContentUnavailableView(search.isEmpty ? (directMessages ? "会話をはじめよう" : "チャンネルはまだありません") : "見つかりませんでした",
                                           systemImage: directMessages ? "bubble.left.and.bubble.right" : "number",
                                           description: Text(search.isEmpty ? "右上の ＋ から作成できます。" : "別のキーワードで検索してください。"))
                } else {
                    channelSection("参加中", values: filtered.filter { store.joined($0.id) })
                    if !directMessages { channelSection("チャンネルを探す", values: filtered.filter { !store.joined($0.id) }) }
                }
            }
            .navigationTitle(directMessages ? "メッセージ" : "ホーム")
            .searchable(text: $search, prompt: directMessages ? "会話を検索" : "チャンネルを検索")
            .refreshable { store.reconnect(); await store.refreshReadStates() }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(directMessages ? "DM を始める" : "チャンネルを作成", systemImage: "plus") { creating = true }
                        .accessibilityIdentifier("createButton")
                }
            }
            .navigationDestination(for: ChannelRoute.self) { route in ChannelScreen(route: route) }
            .sheet(isPresented: $creating) {
                if directMessages { PersonPicker { path.append(ChannelRoute(channel: $0)) } }
                else { CreateChannelScreen { path.append(ChannelRoute(channel: $0)) } }
            }
        }
    }

    @ViewBuilder private func channelSection(_ title: String, values: [Channel]) -> some View {
        if !values.isEmpty {
            Section(title) {
                ForEach(values) { channel in
                    NavigationLink(value: ChannelRoute(channel: channel)) {
                        HStack(spacing: 13) {
                            if directMessages { Avatar(name: store.title(channel)) }
                            else {
                                Image(systemName: channel.is_private ? "lock" : "number")
                                    .font(.title3.weight(.medium)).foregroundStyle(.secondary)
                                    .frame(width: 36, height: 40)
                            }
                            VStack(alignment: .leading, spacing: 3) {
                                Text(store.title(channel)).fontWeight(store.unread.contains(channel.id) ? .bold : .medium)
                                if let description = channel.description, !description.isEmpty {
                                    Text(description).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                }
                            }
                            Spacer()
                            if store.unread.contains(channel.id) {
                                Circle().fill(Color.zooAccent).frame(width: 8, height: 8).accessibilityLabel("未読あり")
                            }
                        }.padding(.vertical, 4)
                    }.accessibilityIdentifier("channel-\(channel.id)")
                }
            }
        }
    }
}

struct CreateChannelScreen: View {
    @Environment(ZooStore.self) private var store
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
                            _ = await store.perform {
                                let channel = try await store.createChannel(name: name.trimmingCharacters(in: .whitespacesAndNewlines), description: description, isPrivate: isPrivate)
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

struct PersonPicker: View {
    @Environment(ZooStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let selected: (Channel) -> Void
    @State private var search = ""
    @State private var busy = false

    var body: some View {
        NavigationStack {
            List(store.people.filter { !$0.is_bot && $0.id != store.me?.id && (search.isEmpty || $0.display_name.localizedCaseInsensitiveContains(search)) }) { person in
                Button {
                    busy = true
                    Task {
                        _ = await store.perform { let channel = try await store.startDM(person); dismiss(); selected(channel) }
                        busy = false
                    }
                } label: {
                    HStack(spacing: 12) {
                        Avatar(name: person.display_name)
                        VStack(alignment: .leading) { Text(person.display_name); Text("@\(person.user_name)").font(.caption).foregroundStyle(.secondary) }
                    }.foregroundStyle(.primary)
                }.disabled(busy)
            }
            .searchable(text: $search, prompt: "メンバーを検索")
            .navigationTitle("新しいメッセージ").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() }.disabled(busy) } }
            .interactiveDismissDisabled(busy)
        }
    }
}
