import SwiftUI

struct GlobalSearchScreen: View {
    let workspace: WorkspaceStore

    var body: some View {
        NavigationStack {
            GlobalSearchContent(workspace: workspace, search: workspace.search) { item in
                Task { _ = await workspace.openSearchResultInTab(item.destination) }
            }
            .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
            .navigationTitle("検索")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

struct GlobalSearchContent: View {
    let workspace: WorkspaceStore
    @Bindable var search: GlobalSearchStore
    let open: (SearchItem) -> Void

    var body: some View {
        AppList {
            SearchFilterBar(workspace: workspace, search: search)
            Text("暗号化されたチャンネルは検索の対象外です。")
                .font(.caption).foregroundStyle(.secondary)
                .listRowSeparator(.hidden)
                .accessibilityIdentifier("global-search-encrypted-note")
            if let message = search.validationError {
                Label(message, systemImage: "exclamationmark.circle")
                    .font(.subheadline).foregroundStyle(.red)
                    .listRowSeparator(.hidden)
            }
            states
        }
        .searchable(text: $search.text, placement: .navigationBarDrawer(displayMode: .always), prompt: "メッセージとファイルを検索")
        .onSubmit(of: .search) { search.submit() }
        .overlay {
            if case .loading = search.phase {
                ProgressView("検索中…")
            }
        }
        .accessibilityIdentifier("global-search")
    }

    @ViewBuilder
    private var states: some View {
        switch search.phase {
        case .idle:
            if search.validationError == nil, !search.filters.hasReversedDays() {
                ContentUnavailableView("検索", systemImage: "magnifyingglass",
                                       description: Text("検索する語を入力してください。空白で区切ると、すべての語を含むものを探します。"))
                    .listRowSeparator(.hidden)
                    .accessibilityIdentifier("global-search-prompt")
            }
        case .loading:
            EmptyView()
        case .failed(let message):
            VStack(alignment: .leading, spacing: 8) {
                Label(message, systemImage: "wifi.exclamationmark")
                Button("再試行") { search.retry() }.buttonStyle(.bordered)
                    .accessibilityIdentifier("global-search-retry")
            }
            .listRowSeparator(.hidden)
            results
        case .loaded:
            if search.items.isEmpty {
                empty
            } else {
                results
                footer
            }
        }
    }

    private var empty: some View {
        VStack(spacing: 12) {
            if search.nextCursor == nil {
                ContentUnavailableView("見つかりませんでした。", systemImage: "magnifyingglass")
                    .accessibilityIdentifier("global-search-empty")
            } else {
                ContentUnavailableView {
                    Label("見つかりませんでした", systemImage: "magnifyingglass")
                } description: {
                    Text("\(search.searchedUntil.map { "\($0) までに" } ?? "")見つかりませんでした。さらに古い期間を検索できます。")
                }
                .accessibilityIdentifier("global-search-empty-older")
                olderButton
            }
        }
        .listRowSeparator(.hidden)
    }

    @ViewBuilder
    private var results: some View {
        ForEach(search.items, id: \.rowID) { item in
            Button { open(item) } label: {
                GlobalSearchRow(workspace: workspace, item: item)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("global-search-result-\(item.rowID)")
        }
    }

    @ViewBuilder
    private var footer: some View {
        VStack(spacing: 8) {
            if let until = search.searchedUntil {
                Text("\(until) まで検索しました").font(.caption).foregroundStyle(.secondary)
                    .accessibilityIdentifier("global-search-searched-until")
            }
            if search.nextCursor != nil { olderButton }
        }
        .frame(maxWidth: .infinity)
        .listRowSeparator(.hidden)
    }

    @ViewBuilder
    private var olderButton: some View {
        if search.loadingOlder {
            ProgressView("検索中…")
        } else {
            Button("さらに古い結果を検索") { search.loadOlder() }
                .buttonStyle(.bordered)
                .accessibilityIdentifier("global-search-older")
        }
    }
}

struct GlobalSearchRow: View {
    let workspace: WorkspaceStore
    let item: SearchItem

    private var person: Person? { item.user_id.flatMap { workspace.people[$0] } }
    private var authorName: String { item.author?.name ?? person?.name ?? "不明なユーザー" }
    private var place: String {
        if item.is_dm == true {
            return workspace.channels.first { $0.id == item.channel_id }.map { workspace.name(for: $0) } ?? "ダイレクトメッセージ"
        }
        let name = item.channel_name ?? workspace.channels.first { $0.id == item.channel_id }?.channel_name
        return name.map { "#\($0)" } ?? "チャンネル"
    }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            AvatarView(name: authorName, url: item.author?.avatarURL ?? person?.avatarURL)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(authorName).font(.subheadline.weight(.semibold)).lineLimit(1)
                    if let date = item.createdDate {
                        Text(date.formatted(date: .abbreviated, time: .shortened))
                            .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                if item.isFile {
                    fileLabel
                } else {
                    GlobalSearchSnippet(workspace: workspace, item: item)
                }
                HStack(spacing: 6) {
                    Text(place).lineLimit(1)
                    if item.parent_id != nil {
                        Text("スレッドの返信").padding(.horizontal, 6)
                            .background(AppDesign.accentBackground, in: Capsule())
                    }
                }
                .font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    private var fileLabel: some View {
        HStack(spacing: 8) {
            Image(systemName: "doc").foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.file_name ?? "ファイル").font(.body).lineLimit(2)
                if let size = item.file_size {
                    Text(ByteCountFormatter.string(fromByteCount: size, countStyle: .file))
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }
}

private struct GlobalSearchSnippet: View {
    let workspace: WorkspaceStore
    let item: SearchItem
    @ScaledMetric(relativeTo: .body) private var emojiHeight: CGFloat = 24
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        let parts = MessageContent.parse(item.snippet ?? "", mention: workspace.mentionIdentity) { key in
            let name = String(key.dropFirst().dropLast())
            return item.emojis?[name]?.imageURL(relativeTo: workspace.client.serverURL)
                ?? workspace.emojiImageURL(for: key, message: nil)
        }
        let urls = Array(Set(parts.compactMap { part -> URL? in
            if case .emoji(_, let url) = part { return url }
            return nil
        })).sorted { $0.absoluteString < $1.absoluteString }
        let states = Dictionary(uniqueKeysWithValues: urls.map { ($0, workspace.images.state(for: $0)) })
        let images = states.compactMapValues { $0.inlineImage(height: emojiHeight) }
        MessageContentText.render(parts, images: images, emojiHeight: emojiHeight)
            .lineLimit(3)
            .tint(AppDesign.externalLink)
            .task(id: scenePhase == .active ? urls : []) {
                guard scenePhase == .active, !urls.isEmpty else { return }
                await workspace.images.watch(states, policy: .stored)
            }
    }
}
