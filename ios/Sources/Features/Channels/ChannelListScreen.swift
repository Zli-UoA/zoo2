import SwiftUI

struct ChannelListScreen: View {
    @Environment(ChannelViewModel.self) private var model
    @Environment(SessionController.self) private var session
    let directMessages: Bool
    @State private var search = ""
    @State private var path: [ChannelRoute] = []
    @State private var creating = false
    @State private var browsing = false
    @State private var browseSearch = ""
    @State private var includeArchived = false

    private var filtered: [Channel] {
        model.channels.filter { $0.is_dm == directMessages && $0.deleted_at == nil && $0.archive_at == nil && model.joined($0.id) &&
            (search.isEmpty || model.title($0).localizedCaseInsensitiveContains(search)) }
            .sorted { model.title($0).localizedStandardCompare(model.title($1)) == .orderedAscending }
    }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if filtered.isEmpty {
                    ContentUnavailableView(search.isEmpty ? (directMessages ? "会話をはじめよう" : "参加中のチャンネルはありません") : "見つかりませんでした",
                                           systemImage: directMessages ? "bubble.left.and.bubble.right" : "number",
                                           description: Text(search.isEmpty ? (directMessages ? "右上の ＋ からメンバーを選べます。" : "チャンネルを探して、会話に参加しましょう。") : "別のキーワードで検索してください。"))
                } else {
                    channelSection(directMessages ? "会話" : "参加中", values: filtered)

                }
                if !directMessages {
                    Section {
                        Button { browsing = true } label: {
                            Label("チャンネルを探す", systemImage: "magnifyingglass")
                                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                                .contentShape(Rectangle())
                        }
                        .accessibilityIdentifier("browseChannels")
                        .listRowBackground(Color.zooBackground)
                    }
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(Color.zooSidebar)
            .navigationTitle(directMessages ? "DM" : "チャンネル")
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $search, prompt: directMessages ? "会話を検索" : "チャンネルを検索")
            .refreshable { session.reconnect() }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(directMessages ? "DM を始める" : "チャンネルを作成", systemImage: "plus") { creating = true }
                        .accessibilityIdentifier("createButton")
                }
            }
            .navigationDestination(for: ChannelRoute.self) { route in ChannelScreen(route: route) }
            .sheet(isPresented: $browsing) { channelBrowser }
            .sheet(isPresented: $creating) {
                if directMessages { PersonPicker { path.append(ChannelRoute(channel: $0)) } }
                else { CreateChannelScreen { path.append(ChannelRoute(channel: $0)) } }
            }
        }
    }

    private var channelBrowser: some View {
        NavigationStack {
            List {
                Section {
                    Toggle("アーカイブ済みも表示", isOn: $includeArchived)
                        .accessibilityIdentifier("includeArchived")
                }
                let matches = model.channels.filter {
                    !$0.is_dm && $0.deleted_at == nil && (includeArchived || $0.archive_at == nil) &&
                    (browseSearch.isEmpty || model.title($0).localizedCaseInsensitiveContains(browseSearch))
                }.sorted { model.title($0).localizedStandardCompare(model.title($1)) == .orderedAscending }
                if matches.isEmpty {
                    ContentUnavailableView.search(text: browseSearch)
                } else {
                    channelSection("チャンネル", values: matches)
                }
            }
            .listStyle(.plain)
            .accessibilityIdentifier("channelBrowserList")
            .navigationTitle("チャンネルを探す").navigationBarTitleDisplayMode(.inline)
            .searchable(text: $browseSearch, prompt: "チャンネルを検索")
            .navigationDestination(for: ChannelRoute.self) { ChannelScreen(route: $0) }
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("閉じる") { browsing = false } }
            }
        }
    }

    @ViewBuilder private func channelSection(_ title: String, values: [Channel]) -> some View {
        if !values.isEmpty {
            Section(title) {
                ForEach(values) { channel in
                    NavigationLink(value: ChannelRoute(channel: channel)) {
                        HStack(spacing: 12) {
                            if directMessages { Avatar(name: model.title(channel)) }
                            else {
                                Image(systemName: channel.is_private ? "lock" : "number")
                                    .font(.system(size: 18, weight: .medium))
                                    .foregroundStyle(model.unread.contains(channel.id) ? Color.zooAccent : .secondary)
                                    .frame(width: 36, height: 36)
                                    .background(model.unread.contains(channel.id) ? Color.zooBrandMuted : Color.zooMuted,
                                                in: RoundedRectangle(cornerRadius: 10))
                            }
                            VStack(alignment: .leading, spacing: 3) {
                                Text(model.title(channel)).font(.body).fontWeight(model.unread.contains(channel.id) ? .bold : .medium)
                                if let description = channel.description, !description.isEmpty {
                                    Text(description).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                }
                            }
                            Spacer()
                            if channel.archive_at != nil {
                                Image(systemName: "archivebox").foregroundStyle(.secondary)
                                    .accessibilityLabel("アーカイブ済み")
                            }
                            if model.unread.contains(channel.id) {
                                Circle().fill(Color.zooAccent).frame(width: 8, height: 8).accessibilityLabel("未読あり")
                            }
                        }.padding(.vertical, 6)
                    }.listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                        .accessibilityIdentifier("channel-\(channel.id)")
                }
            }
        }
    }
}
