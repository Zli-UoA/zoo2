import AVFAudio
import SwiftUI

struct WorkspaceView: View {
    @Bindable var workspace: WorkspaceStore
    let session: AppSession
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    var body: some View {
        Group {
            if UIDevice.current.userInterfaceIdiom == .pad, horizontalSizeClass == .regular {
                TabletWorkspaceView(workspace: workspace, session: session)
            } else {
                compactTabs
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
        .sheet(isPresented: Binding(get: { workspace.calls.showingCall }, set: { workspace.calls.showingCall = $0 })) {
            CallView(store: workspace.calls, workspace: workspace)
        }
        .onReceive(NotificationCenter.default.publisher(for: AVAudioSession.interruptionNotification)) { notification in
            if let type = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
               type == AVAudioSession.InterruptionType.began.rawValue,
               workspace.systemCalls?.isManagingCall != true { workspace.calls.finish() }
        }
        .task(id: scenePhase) {
            if scenePhase == .active, workspace.systemCalls == nil { await workspace.calls.runInvitations() }
        }
        .modifier(ConversationSync(workspace: workspace))
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.calls.runActiveCalls() }
        }
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.run() }
        }
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.inbox.run() }
        }
    }

    private var compactTabs: some View {
        TabView(selection: $workspace.selectedTab) {
            Tab("チャンネル", systemImage: "number", value: WorkspaceTab.channels) {
                chatStack(path: $workspace.channelPath, directMessages: false)
            }
            Tab("DM", systemImage: "bubble.left.and.bubble.right.fill", value: WorkspaceTab.directMessages) {
                chatStack(path: $workspace.dmPath, directMessages: true)
            }
            Tab("通知", systemImage: "bell.fill", value: WorkspaceTab.notifications) {
                NavigationStack {
                    InboxView(store: workspace.inbox, workspace: workspace,
                              isActive: workspace.selectedTab == .notifications && scenePhase == .active,
                              openChat: workspace.openInboxMessage)
                }
            }
            .badge(workspace.inbox.unreadCount)
            Tab("プロフィール", systemImage: "person.crop.circle.fill", value: WorkspaceTab.profile) {
                NavigationStack { ProfileView(workspace: workspace, session: session) }
            }
        }
    }

    private func chatStack(path: Binding<[ChatRoute]>, directMessages: Bool) -> some View {
        NavigationStack(path: path) {
            ConversationDirectory(workspace: workspace, directMessages: directMessages)
                .navigationDestination(for: ChatRoute.self) { route in
                    ConversationDestination(workspace: workspace, route: route)
                }
        }
    }
}

struct ConversationDestination: View {
    let workspace: WorkspaceStore
    let route: ChatRoute

    var body: some View {
        if let channel = workspace.channels.first(where: { $0.id == route.channelID }) {
            ConversationView(store: workspace.conversation(for: channel), workspace: workspace, route: route)
        } else {
            ContentUnavailableView("会話を表示できません", systemImage: "bubble.left.and.exclamationmark.bubble.right")
        }
    }
}

struct ConversationDirectory: View {
    let workspace: WorkspaceStore
    let directMessages: Bool
    var isSidebar = false
    @Environment(\.scenePhase) private var scenePhase
    @State private var search = ""
    @State private var unreadOnly = false
    @State private var creating = false
    @State private var browsing = false
    @State private var pinnedExpanded = true
    @State private var listExpanded = true

    private var showsThreadIndex: Bool {
        !directMessages && workspace.selectedTab == .channels && (isSidebar || workspace.channelPath.isEmpty) && scenePhase == .active
    }

    private var visible: [Channel] {
        workspace.channels.filter {
            $0.is_dm == directMessages && $0.archive_at == nil && (workspace.joinedChannelIDs.contains($0.id) || workspace.pinnedChannelIDs.contains($0.id)) &&
            (!unreadOnly || workspace.unread.contains($0.id)) &&
            (search.isEmpty || workspace.name(for: $0).localizedStandardContains(search) ||
             ($0.description ?? "").localizedStandardContains(search))
        }
    }

    var body: some View {
        AppList {
            if let error = workspace.error {
                Label(error, systemImage: "wifi.exclamationmark")
                Button("再読み込み") { Task { await workspace.refresh() } }
            }
            let channels = visible
            if !workspace.hasLoaded && workspace.loading {
                ProgressView("読み込み中…")
            } else if channels.isEmpty {
                Group {
                    if !search.isEmpty { ContentUnavailableView.search(text: search) }
                    else if unreadOnly { ContentUnavailableView("未読の会話はありません", systemImage: "checkmark.circle") }
                    else if directMessages {
                        ContentUnavailableView {
                            Label("DMはまだありません", systemImage: "bubble.left.and.bubble.right")
                        } actions: { Button("DMを始める") { creating = true } }
                    } else {
                        ContentUnavailableView {
                            Label("参加中のチャンネルはありません", systemImage: "number")
                        } actions: { Button("チャンネルを探す") { browsing = true } }
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
                Button("ピン留めを再読み込み") { Task { await workspace.loadPins() } }
            }
            if !directMessages { ChannelThreadPagination(store: workspace.threads) }
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
                Button("新しいDM", systemImage: "square.and.pencil") { creating = true }
                    .accessibilityIdentifier("new-dm")
            } else {
                Menu("チャンネルの操作", systemImage: "plus") {
                    Button("チャンネルを探す", systemImage: "magnifyingglass") { browsing = true }
                    Button("チャンネルを作成", systemImage: "number") { creating = true }
                }.accessibilityIdentifier("new-conversation")
            }
        }
        .sheet(isPresented: $creating) {
            if directMessages { PeopleView(workspace: workspace) }
            else { CreateChannelView(workspace: workspace) }
        }
        .sheet(isPresented: $browsing) { ChannelBrowser(workspace: workspace) }
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
    }

    @ViewBuilder
    private func channelLink(_ channel: Channel) -> some View {
        if isSidebar {
            Button { workspace.addAndOpen(channel, joined: false) } label: {
                channelLabel(channel).contentShape(Rectangle())
            }
                .buttonStyle(.plain)
                .listRowBackground(workspace.path.first?.channelID == channel.id ? Color.accentColor.opacity(0.12) : nil)
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
                    .background(Color(.secondarySystemBackground), in: Circle())
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
            }
        }
    }

    private func channelRows(_ channels: [Channel]) -> some View {
        ForEach(channels) { channel in
            channelLink(channel)
                .listRowInsets(EdgeInsets(top: directMessages ? 10 : 8, leading: 12,
                                         bottom: directMessages ? 10 : 8, trailing: 12))
                .accessibilityIdentifier("channel-\(channel.id)")
                .contextMenu { ChannelPinButton(channel: channel, workspace: workspace) }
                .accessibilityHint("長押しでピン留めを変更できます")
            if !directMessages, channel.encrypted != true {
                ChannelThreadRows(store: workspace.threads, channelID: channel.id) { parentID in
                    workspace.openListedThread(parentID, in: channel)
                }
            }
        }
    }
}

private struct ChannelBrowser: View {
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""
    @State private var includeArchived = false
    @State private var opening: Channel?
    @State private var error: String?

    private var matches: [Channel] {
        workspace.channels.filter { channel in
            guard !channel.is_dm,
                  search.isEmpty || channel.channel_name.localizedStandardContains(search) else { return false }
            let joined = workspace.joinedChannelIDs.contains(channel.id)
            if channel.archive_at != nil { return includeArchived && (!channel.is_private || joined) }
            return !channel.is_private && !joined
        }
    }

    var body: some View {
        NavigationStack {
            AppList {
                Toggle("アーカイブ済みを含める", isOn: $includeArchived)
                if let error { Text(error).foregroundStyle(.red) }
                if matches.isEmpty { Text("該当するチャンネルはありません").foregroundStyle(.secondary) }
                ForEach(matches) { channel in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(channel.channel_name)
                            if let description = channel.description { Text(description).font(.caption).foregroundStyle(.secondary) }
                        }
                        Spacer()
                        Button(channel.archive_at == nil ? "参加" : "開く") { opening = channel }
                            .buttonStyle(.bordered).buttonBorderShape(.capsule).controlSize(.small)
                            .accessibilityIdentifier("browse-channel-\(channel.id)")
                    }
                }
            }
            .disabled(opening != nil)
            .navigationTitle("チャンネルを探す")
            .navigationBarTitleDisplayMode(.inline)
            .appSearch(text: $search, prompt: "チャンネル名を検索", identifier: "channel-browser")
            .toolbar { ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } } }
            .task(id: opening?.id) {
                guard let channel = opening else { return }
                defer { opening = nil }
                do {
                    if channel.archive_at == nil { try await workspace.client.joinChannel(id: channel.id) }
                    try Task.checkCancellation()
                    workspace.addAndOpen(channel, joined: channel.archive_at == nil)
                    dismiss()
                } catch {
                    if !Task.isCancelled { self.error = error.localizedDescription; workspace.report(error) }
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

    private struct TypingScope: Equatable {
        let channelID: String
        let parentID: String?
    }

    private struct MessageScope: Equatable {
        let channelID: String
        let requestID: UUID
        let parentID: String?
        let focusID: String?
    }

    private var typingScope: TypingScope? {
        guard let store = activeConversation, store.canPost, let route = workspace.path.last else { return nil }
        switch route {
        case .channel(let id): return TypingScope(channelID: id, parentID: nil)
        case .thread(let id, let parentID, _):
            guard store.message(id: parentID) != nil else { return nil }
            return TypingScope(channelID: id, parentID: parentID)
        }
    }

    private var activeConversation: ConversationStore? {
        guard scenePhase == .active, let id = workspace.path.last?.channelID,
              let channel = workspace.channels.first(where: { $0.id == id }) else { return nil }
        return workspace.conversation(for: channel)
    }

    private var feedParentID: String? {
        if case .thread(_, let parent, _) = workspace.path.last { return parent }
        return nil
    }

    func body(content: Content) -> some View {
        let store = activeConversation
        let readable = store.flatMap { !$0.blocked && $0.channel.encrypted != true ? $0 : nil }
        let attachments = readable.flatMap { $0.needsAttachmentStream ? $0 : nil }
        let reactions = readable.flatMap { $0.needsReactionStream ? $0 : nil }
        let authors = readable.map { Authors(channelID: $0.channel.id, ids: Set($0.messages.map(\.user_id))) }
        let needsEmojiCatalog = scenePhase == .active && (store != nil || workspace.selectedTab == .notifications)
        let typing = typingScope
        let messageScope = readable.map { MessageScope(channelID: $0.channel.id, requestID: $0.syncRequest,
                                                       parentID: feedParentID, focusID: $0.focusedMessageID) }
        content
            .task(id: typing) {
                if let typing, let store { await store.typing.run(parentID: typing.parentID) }
            }
            .task(id: messageScope) {
                guard let readable, let messageScope else { return }
                readable.selectFeed(parentID: messageScope.parentID)
                await readable.run()
            }
            .task(id: readable?.channel.id) { if let readable { await workspace.loadMembers(channelID: readable.channel.id) } }
            .task(id: attachments?.syncRequest) { await attachments?.runAttachments() }
            .task(id: reactions?.syncRequest) { await reactions?.runReactions() }
            .task(id: readable?.reactionResyncTicket) {
                guard let readable, let ticket = readable.reactionResyncTicket else { return }
                do { try await Task.sleep(for: .seconds(15)) } catch { return }
                if readable.reactionResyncTicket == ticket { readable.retryAccess() }
            }
            .task(id: needsEmojiCatalog) {
                guard needsEmojiCatalog else { return }
                async let emojis: Void = workspace.loadEmojis()
                async let accounts: Void = workspace.loadEmojiAccounts()
                _ = await (emojis, accounts)
            }
            .task(id: authors) {
                if let readable { await workspace.resolveAuthors(in: readable.messages) }
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
            AppForm {
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
                ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } }
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
    @State private var recipients: Set<String> = []
    @State private var submitting = false
    @State private var error: String?

    private var candidates: [Person] {
        workspace.people.values.filter {
            $0.id != workspace.user.id && !$0.is_bot && !$0.is_deactivated &&
            (recipients.contains($0.id) || search.isEmpty || $0.name.localizedStandardContains(search) ||
             $0.user_name.localizedStandardContains(search))
        }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    var body: some View {
        NavigationStack {
            AppList {
                if let error { Text(error).foregroundStyle(.red) }
                Section("宛先（9人まで）") {
                    ForEach(candidates) { person in
                        let selected = recipients.contains(person.id)
                        Button {
                            if selected { recipients.remove(person.id) }
                            else { recipients.insert(person.id) }
                        } label: {
                            HStack {
                                PersonRow(person: person).foregroundStyle(.primary)
                                Spacer()
                                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                            }
                        }
                        .disabled(!selected && recipients.count >= 9)
                        .accessibilityAddTraits(selected ? .isSelected : [])
                        .accessibilityIdentifier("dm-recipient-\(person.id)")
                    }
                }
            }
            .disabled(submitting)
            .appSearch(text: $search, prompt: "名前を検索", identifier: "people") {
                Button("開始") { submitting = true }
                    .disabled(recipients.isEmpty || submitting)
                    .accessibilityIdentifier("start-dm")
            }
            .navigationTitle("新しいDM").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } }
            }
            .task(id: submitting) {
                guard submitting else { return }
                defer { submitting = false }
                do {
                    let ids = recipients.sorted()
                    let channel = try await workspace.client.directMessage(userIDs: ids)
                    try Task.checkCancellation()
                    workspace.addAndOpen(channel, members: ids + [workspace.user.id])
                    dismiss()
                } catch {
                    if !Task.isCancelled { self.error = error.localizedDescription; workspace.report(error) }
                }
            }
        }
    }
}
