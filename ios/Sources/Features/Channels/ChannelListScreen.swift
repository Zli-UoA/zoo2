import SwiftUI

struct ChannelListScreen: View {
    @Environment(ChannelViewModel.self) private var model
    @Environment(SessionController.self) private var session
    let directMessages: Bool
    @State private var search = ""
    @State private var path: [ChannelRoute] = []
    @State private var creating = false
    @State private var includeArchived = false

    private var filtered: [Channel] {
        model.channels.filter { $0.is_dm == directMessages && $0.deleted_at == nil && (includeArchived || $0.archive_at == nil) &&
            (search.isEmpty || model.title($0).localizedCaseInsensitiveContains(search)) }
            .sorted { model.title($0).localizedStandardCompare(model.title($1)) == .orderedAscending }
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
                    channelSection("参加中", values: filtered.filter { model.joined($0.id) })
                    if !directMessages { channelSection("チャンネルを探す", values: filtered.filter { !model.joined($0.id) }) }
                }
            }
            .navigationTitle(directMessages ? "メッセージ" : "ホーム")
            .searchable(text: $search, prompt: directMessages ? "会話を検索" : "チャンネルを検索")
            .refreshable { session.reconnect() }
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
                            if directMessages { Avatar(name: model.title(channel)) }
                            else {
                                Image(systemName: channel.is_private ? "lock" : "number")
                                    .font(.title3.weight(.medium)).foregroundStyle(.secondary)
                                    .frame(width: 36, height: 40)
                            }
                            VStack(alignment: .leading, spacing: 3) {
                                Text(model.title(channel)).fontWeight(model.unread.contains(channel.id) ? .bold : .medium)
                                if let description = channel.description, !description.isEmpty {
                                    Text(description).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                }
                            }
                            Spacer()
                            if model.unread.contains(channel.id) {
                                Circle().fill(Color.zooAccent).frame(width: 8, height: 8).accessibilityLabel("未読あり")
                            }
                        }.padding(.vertical, 4)
                    }.accessibilityIdentifier("channel-\(channel.id)")
                }
            }
        }
    }
}
