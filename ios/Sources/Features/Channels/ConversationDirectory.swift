import SwiftUI

struct ConversationDirectory: View {
    let workspace: WorkspaceStore
    let directMessages: Bool
    var isSidebar = false
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.colorScheme) private var colorScheme
    @State private var search = ""
    @State private var unreadOnly = false
    @State private var pinnedExpanded = true
    @State private var listExpanded = true

    private var showsThreadIndex: Bool {
        !directMessages && workspace.selectedTab == .channels && (isSidebar || workspace.channelPath.isEmpty) && scenePhase == .active
    }

    private var canReopenChannel: Bool { !isSidebar && !directMessages && workspace.resumableChannel != nil }

    private var visible: [Channel] {
        guard workspace.pinsReady else { return [] }
        return workspace.channels.filter {
            $0.is_dm == directMessages && $0.archive_at == nil && (workspace.joinedChannelIDs.contains($0.id) || workspace.pinnedChannelIDs.contains($0.id)) &&
            (!unreadOnly || workspace.unread.contains($0.id)) &&
            (search.isEmpty || workspace.name(for: $0).localizedStandardContains(search) ||
             ($0.description ?? "").localizedStandardContains(search))
        }
    }

    var body: some View {
        AppList {
            if let error = workspace.error, !workspace.loading || workspace.hasLoaded {
                Label(error, systemImage: "wifi.exclamationmark")
            }
            let channels = visible
            if channels.isEmpty && workspace.directoryReady {
                Group {
                    if !search.isEmpty { ContentUnavailableView.search(text: search) }
                    else if unreadOnly { ContentUnavailableView("未読の会話はありません", systemImage: "checkmark.circle") }
                    else if directMessages {
                        ContentUnavailableView {
                            Label("DMはまだありません", systemImage: "bubble.left.and.bubble.right")
                        } actions: { Button("DMを始める") { workspace.presentedSheet = .createDirectMessage } }
                    } else {
                        ContentUnavailableView {
                            Label("参加中のチャンネルはありません", systemImage: "number")
                        } actions: { Button("チャンネルを探す") { workspace.presentedSheet = .browseChannels } }
                    }
                }
                .listRowSeparator(.hidden)
            }
            let pinned = workspace.pinnedChannelIDs.compactMap { id in channels.first { $0.id == id } }
            let unpinned = channels.filter { !workspace.pinnedChannelIDs.contains($0.id) }
            if !pinned.isEmpty {
                channelSection("ピン留め", id: "pinned", channels: pinned, isExpanded: $pinnedExpanded)
            }
            if !unpinned.isEmpty {
                channelSection("一覧", id: "list", channels: unpinned, isExpanded: $listExpanded)
            }
            if let error = workspace.pinsError {
                Text(error).font(.caption).foregroundStyle(.secondary)
            }
            if !directMessages {
                ChannelThreadPagination(store: workspace.threads)
                    .listRowSeparator(.hidden)
            }
        }
        .overlay {
            if workspace.directoryLoading {
                ProgressView("読み込み中…")
                    .accessibilityIdentifier("directory-loading")
            }
        }
        .simultaneousGesture(DragGesture(minimumDistance: 30).onEnded { drag in
            guard canReopenChannel, drag.translation.width < -80,
                  abs(drag.translation.height) < abs(drag.translation.width) * 0.5 else { return }
            withAnimation(.snappy) { workspace.reopenLastChannel() }
        }, including: canReopenChannel ? .all : .subviews)
        .accessibilityActions {
            if canReopenChannel, let channel = workspace.resumableChannel {
                Button("前のチャンネルを開く：\(workspace.name(for: channel))") { workspace.reopenLastChannel() }
            }
        }
        .navigationTitle(directMessages ? "DM" : "チャンネル")
        .navigationBarTitleDisplayMode(isSidebar ? .inline : .large)
        .safeAreaInset(edge: .top, spacing: 0) {
            VStack(spacing: 0) {
                if let error = workspace.pinUpdateError {
                    Label(error, systemImage: "exclamationmark.circle")
                        .font(.caption).foregroundStyle(.red)
                        .frame(maxWidth: .infinity, alignment: .leading).padding(12)
                        .background(.regularMaterial)
                }
                if let error = workspace.readUpdateError {
                    Label(error, systemImage: "exclamationmark.circle")
                        .font(.caption).foregroundStyle(.red)
                        .frame(maxWidth: .infinity, alignment: .leading).padding(12)
                        .background(.regularMaterial)
                        .accessibilityIdentifier("channel-read-error")
                }
            }
        }
        .refreshable {
            await workspace.refresh()
            await workspace.loadPins()
            await workspace.calls.refreshActiveCalls()
            if showsThreadIndex { await workspace.threads.refresh() }
        }
        .task(id: showsThreadIndex) { if showsThreadIndex { await workspace.threads.run() } }
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button("未読のみ", systemImage: unreadOnly ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle") {
                    unreadOnly.toggle()
                }.accessibilityValue(unreadOnly ? "有効" : "無効")
            }
        }
        .appSearch(text: $search, prompt: directMessages ? "DMを検索" : "チャンネルを検索", identifier: "conversation") {
            if directMessages {
                Button("新しいDM", systemImage: "square.and.pencil") { workspace.presentedSheet = .createDirectMessage }
                    .accessibilityIdentifier("new-dm")
            } else {
                Menu("チャンネルの操作", systemImage: "plus") {
                    Button("チャンネルを探す", systemImage: "magnifyingglass") { workspace.presentedSheet = .browseChannels }
                    Button("チャンネルを作成", systemImage: "number") { workspace.presentedSheet = .createChannel }
                }.accessibilityIdentifier("new-conversation")
            }
        }
    }

    private func channelSection(_ title: LocalizedStringKey, id: String, channels: [Channel],
                                isExpanded: Binding<Bool>) -> some View {
        let expanded = isExpanded.wrappedValue || !search.isEmpty
        return Section {
            if expanded { channelRows(channels) }
        } header: {
            Button {
                withAnimation(.snappy) { isExpanded.wrappedValue.toggle() }
            } label: {
                HStack {
                    Text(title)
                    Spacer()
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.caption.weight(.semibold))
                        .accessibilityHidden(true)
                }
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Color(.secondaryLabel))
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!search.isEmpty)
            .accessibilityValue(expanded ? "展開中" : "折り畳み中")
            .accessibilityIdentifier("channel-section-\(id)")
            .listRowInsets(EdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12))
        }
        .listSectionSeparator(.hidden)
    }

    @ViewBuilder
    private func channelLink(_ channel: Channel) -> some View {
        if isSidebar {
            Button { workspace.addAndOpen(channel, joined: false) } label: {
                channelLabel(channel).contentShape(Rectangle())
            }
                .buttonStyle(.plain)
                .listRowBackground(workspace.path.first?.channelID == channel.id ? AppDesign.selectionBackground
                                   : (colorScheme == .dark ? AppDesign.background : nil))
                .accessibilityAddTraits(workspace.path.first?.channelID == channel.id ? .isSelected : [])
        } else {
            NavigationLink(value: ChatRoute.channel(channel.id)) { channelLabel(channel) }
        }
    }

    private func channelLabel(_ channel: Channel) -> some View {
        HStack(spacing: 12) {
            if let peer = workspace.directMessagePeer(for: channel) {
                AvatarView(name: peer.name, url: peer.avatarURL)
            } else {
                Image(systemName: channel.symbol)
                    .font(.body.weight(.medium)).foregroundStyle(.secondary)
                    .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                    .frame(width: AvatarView.size, height: AvatarView.size)
                    .background(AppDesign.surface, in: Circle())
            }
            let title = HStack(alignment: .firstTextBaseline, spacing: 4) {
                Text(workspace.name(for: channel))
                    .fontWeight(workspace.unread.contains(channel.id) ? .semibold : .regular)
                if directMessages, channel.encrypted == true {
                    Image(systemName: "lock.shield")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize()
                        .accessibilityLabel("暗号化された会話")
                }
            }
            let description = channel.description ?? ""
            let labels = VStack(alignment: .leading, spacing: 2) {
                title
                if !directMessages || !description.isEmpty {
                    Text(description).font(.subheadline).foregroundStyle(.secondary)
                        .lineLimit(1, reservesSpace: true)
                        .accessibilityHidden(description.isEmpty)
                }
            }
            if !directMessages && description.isEmpty {
                // 説明の有無で行の高さが変わらないよう、2段分の寸法を残す。
                labels.hidden().overlay(alignment: .leading) { title }
            } else {
                labels
            }
            Spacer(minLength: 4)
            if workspace.calls.hasActiveCall(in: channel.id) {
                Image(systemName: "headphones")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.green)
                    .fixedSize()
                    .accessibilityLabel("通話中")
                    .accessibilityIdentifier("channel-active-call-\(channel.id)")
            }
            if workspace.unread.contains(channel.id) {
                Circle().fill(.tint).frame(width: 8, height: 8).accessibilityLabel("未読")
                    .accessibilityIdentifier("channel-unread-\(channel.id)")
            }
        }
    }

    private func channelRows(_ channels: [Channel]) -> some View {
        ForEach(channels) { channel in
            channelLink(channel)
                .listRowSeparator(.hidden)
                .listRowInsets(EdgeInsets(top: directMessages ? 10 : 8, leading: 12,
                                         bottom: directMessages ? 10 : 8, trailing: 12))
                .accessibilityIdentifier("channel-\(channel.id)")
                .contextMenu {
                    if !directMessages, channel.encrypted != true {
                        Button("すべて既読にする", systemImage: "checkmark.circle") {
                            Task { await workspace.markChannelRead(channel) }
                        }
                        .disabled(!workspace.unread.contains(channel.id) || workspace.markingReadChannelIDs.contains(channel.id))
                        .accessibilityIdentifier("mark-channel-read-\(channel.id)")
                    }
                    ChannelPinButton(channel: channel, workspace: workspace)
                }
                .accessibilityHint(!directMessages && channel.encrypted != true
                    ? "長押しで既読やピン留めを変更できます" : "長押しでピン留めを変更できます")
            if !directMessages, channel.encrypted != true {
                ChannelThreadRows(store: workspace.threads, channelID: channel.id) { parentID in
                    workspace.openListedThread(parentID, in: channel)
                }
                .listRowSeparator(.hidden)
            }
        }
    }
}
