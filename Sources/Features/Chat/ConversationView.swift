import SwiftUI

struct ConversationView: View {
    @Bindable var store: ConversationStore
    let workspace: WorkspaceStore
    let route: ChatRoute
    @FocusState private var composerFocused: Bool
    @State private var showingDetails = false
    @State private var didRequestInitialFocus = false

    private var parentID: String? {
        if case .thread(_, let parent, _) = route { return parent }
        return nil
    }

    private var isVisible: Bool { workspace.path.last == route }

    private var threadUnavailable: Bool {
        guard let parentID else { return false }
        return store.blocked || store.channel.encrypted == true ||
            (!store.loading && store.syncError == nil && store.message(id: parentID) == nil)
    }

    var body: some View {
        VStack(spacing: 0) {
            if parentID == nil, let description = store.channel.description, !description.isEmpty {
                Text(description).font(.caption).foregroundStyle(.secondary)
                    .lineLimit(2).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal).padding(.bottom, 8)
            }
            if store.channel.encrypted == true {
                ContentUnavailableView("暗号化された会話", systemImage: "lock.shield",
                                       description: Text("この iOS 版では暗号化された会話をまだ開けません。対応済みの Zoo クライアントをご利用ください。"))
            } else {
                if let error = store.syncError ?? store.reactionSyncError ?? store.attachmentSyncError {
                    HStack {
                        Label(error, systemImage: "wifi.exclamationmark").font(.caption)
                        Spacer()
                        Button("再試行") { store.retryAccess() }.font(.caption.bold())
                    }.padding(12).background(.orange.opacity(0.12))
                }
                MessageList(store: store, workspace: workspace, parentID: parentID, isVisible: isVisible,
                            composerFocused: $composerFocused) { id in
                    if parentID == nil {
                        workspace.openThread(id, in: store, focusComposer: store.canPost)
                    } else {
                        composerFocused = store.canPost
                    }
                }
            }
        }
        .background(Color(.systemBackground))
        .navigationTitle(parentID == nil ? workspace.name(for: store.channel) : "スレッド")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $showingDetails) { ChannelDetailsView(channel: store.channel, workspace: workspace) }
        .toolbar {
            if parentID == nil {
                ToolbarItem(placement: .principal) {
                    Button { composerFocused = false; showingDetails = true } label: {
                        HStack(spacing: 4) {
                            Text(workspace.name(for: store.channel)).font(.headline).lineLimit(1)
                            Image(systemName: "chevron.down").font(.caption2)
                        }
                    }
                    .foregroundStyle(.primary)
                    .accessibilityLabel("\(workspace.name(for: store.channel))の詳細")
                    .accessibilityIdentifier("channel-details")
                }
            }
            if parentID == nil, store.canPost, !composerFocused {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("音声通話", systemImage: "headphones") {
                        if workspace.calls.channel == nil { workspace.calls.start(channel: store.channel) }
                        else { workspace.calls.showingCall = true }
                    }.accessibilityIdentifier("start-call")
                }
            }
            if isVisible && composerFocused {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("キーボードを閉じる", systemImage: "keyboard.chevron.compact.down") {
                        composerFocused = false
                    }
                    .accessibilityIdentifier("dismiss-composer-keyboard")
                }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            ComposerView(store: store, workspace: workspace, parentID: parentID, focused: $composerFocused)
        }
        .background {
            if case .thread(_, _, true) = route {
                ThreadAppearanceObserver {
                    guard isVisible, store.canPost, !didRequestInitialFocus else { return }
                    didRequestInitialFocus = true
                    composerFocused = true
                }
                .frame(width: 0, height: 0)
            }
        }
        .onChange(of: isVisible && parentID == nil ? store.focusedThreadID : nil, initial: true) { _, target in
            if let target { workspace.openThread(target, in: store, focusComposer: false) }
        }
        .onChange(of: isVisible && threadUnavailable, initial: true) { _, unavailable in
            if unavailable { workspace.closeThread(route) }
        }
    }
}

// SwiftUI の onAppear は横移動の開始時にも呼ばれるため、返信欄は UIKit の表示完了時にフォーカスする。
private struct ThreadAppearanceObserver: UIViewControllerRepresentable {
    let didAppear: () -> Void

    func makeUIViewController(context: Context) -> Controller { Controller() }

    func updateUIViewController(_ controller: Controller, context: Context) {
        controller.didAppear = didAppear
    }

    final class Controller: UIViewController {
        var didAppear: (() -> Void)?

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            didAppear?()
        }
    }
}

private struct MessageList: View {
    @Bindable var store: ConversationStore
    let workspace: WorkspaceStore
    let parentID: String?
    let isVisible: Bool
    @FocusState.Binding var composerFocused: Bool
    let openThread: (String) -> Void
    @Environment(\.scenePhase) private var scenePhase
    @State private var followingLatest = false
    @State private var showsLatestButton = false
    @State private var positionedInitialMessages = false
    @State private var selectedMessage: Message?
    @State private var pressingMessageID: String?
    @State private var replyAfterDismiss: String?
    @State private var wasComposingBeforeMenu = false
    @State private var initialReadState: ChannelReadState?
    @State private var readStateLoaded = false
    @State private var readStateError = false
    @State private var readRetry = 0
    @State private var renderCache = MessageRenderCache()
    @State private var scrollPosition = ScrollPosition(idType: String.self)

    private var firstUnreadID: String? {
        parentID == nil ? MessageTimeline.firstUnread(in: messages, state: initialReadState, userID: store.userID) : nil
    }
    private var positionKey: String {
        "\(messages.last?.id ?? "")|\(readStateLoaded)|\(store.loading)"
    }

    private var messages: [Message] {
        if let parentID { return store.messages.filter { $0.id == parentID || $0.parent_id == parentID } }
        return store.roots
    }

    private var scrollTarget: String? { isVisible ? store.focusedMessageID : nil }

    private var readID: String? {
        guard isVisible, parentID == nil, readStateLoaded, initialReadState != nil, positionedInitialMessages, store.focusedMessageID == nil, followingLatest, scenePhase == .active,
              selectedMessage == nil, replyAfterDismiss == nil else { return nil }
        return messages.last(where: { $0.parent_id == nil })?.id
    }

    private func canOpenThread(for message: Message) -> Bool {
        store.canPost || (parentID == nil && !store.replies(to: message.id).isEmpty)
    }

    private func hasActions(for message: Message) -> Bool {
        !message.content.isEmpty || canOpenThread(for: message)
    }

    private var selectedActionsAvailable: Bool {
        guard let selectedMessage, let current = store.message(id: selectedMessage.id) else { return false }
        return hasActions(for: current)
    }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                if store.loading && messages.isEmpty {
                    ProgressView("メッセージを読み込んでいます…")
                        .frame(maxWidth: .infinity).padding(40)
                } else if messages.isEmpty && store.syncError == nil {
                    ContentUnavailableView("会話を始めましょう", systemImage: "bubble.left.and.bubble.right",
                                           description: Text("最初のメッセージを送ってみませんか。"))
                }
                if readStateError {
                    HStack {
                        Text("未読の位置を取得できませんでした。").font(.caption)
                        Button("再読み込み") { readStateLoaded = false; positionedInitialMessages = false; readRetry += 1 }
                    }.padding().accessibilityIdentifier("read-state-error")
                }
                if let error = store.actionError {
                    Text(error).font(.callout).foregroundStyle(.red).padding()
                }
                ForEach(Array(messages.enumerated()), id: \.element.id) { index, message in
                    VStack(spacing: 0) {
                        if MessageTimeline.startsDay(message, after: index > 0 ? messages[index - 1] : nil), let date = message.date {
                            timelineDivider(date.formatted(.dateTime.year().month().day().weekday().locale(Locale(identifier: "ja_JP"))), color: .primary)
                                .accessibilityIdentifier("date-divider-\(message.id)")
                        }
                        if firstUnreadID == message.id {
                            timelineDivider("ここから未読", color: .red).accessibilityIdentifier("unread-divider")
                        }
                        messageRow(message)
                    }.id(message.id)
                }
            }
            .scrollTargetLayout()
            .padding(.vertical, 8)
        }
        .accessibilityIdentifier(parentID == nil ? "message-list" : "thread-message-list")
        .scrollBounceBehavior(.basedOnSize)
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .scrollPosition($scrollPosition)
        .scrollDismissesKeyboard(.interactively)
        .onScrollTargetVisibilityChange(idType: String.self, threshold: 0.01) { ids in
            renderCache.retain(visibleIDs: ids, orderedIDs: messages.map(\.id))
        }
        .onChange(of: messages.map(\.id)) { _, ids in renderCache.updateMessages(ids) }
        .onScrollGeometryChange(for: MessageScrollState.self) { geometry in
            MessageScrollState(distanceToBottom: geometry.contentSize.height - geometry.contentOffset.y - geometry.containerSize.height)
        } action: { _, state in
            if isVisible {
                followingLatest = state.followingLatest
                showsLatestButton = state.showsLatestButton
            }
        }
        .onChange(of: positionKey, initial: true) { _, _ in
            guard isVisible, !messages.isEmpty, !store.loading, parentID != nil || readStateLoaded else { return }
            if let focused = scrollTarget, messages.contains(where: { $0.id == focused }) {
                positionedInitialMessages = true
                scrollPosition.scrollTo(id: focused, anchor: .center)
                store.focusedMessageID = nil
            } else if !positionedInitialMessages, let firstUnreadID {
                positionedInitialMessages = true
                scrollPosition.scrollTo(id: firstUnreadID, anchor: .top)
            } else if !positionedInitialMessages || followingLatest {
                positionedInitialMessages = true
                if parentID != nil, let last = messages.last {
                    // 表示直後にキーボードが開くスレッドでは、実在する行を初期位置の基準にする。
                    scrollPosition.scrollTo(id: last.id, anchor: .bottom)
                } else {
                    // 行や目印への移動では外側の余白が残るため、バウンドの着地点と同じ下端を指定する。
                    scrollPosition.scrollTo(edge: .bottom)
                }
            }
        }
        .onChange(of: scrollTarget, initial: true) { _, id in
            if let id, messages.contains(where: { $0.id == id }) {
                positionedInitialMessages = true
                scrollPosition.scrollTo(id: id, anchor: .center)
                store.focusedMessageID = nil
            }
        }
        .overlay(alignment: .bottomTrailing) {
            if positionedInitialMessages, showsLatestButton, !messages.isEmpty {
                Button {
                    store.focusedMessageID = nil
                    withAnimation { scrollPosition.scrollTo(edge: .bottom) }
                } label: {
                    Image(systemName: "arrow.down").frame(width: 44, height: 44)
                }
                .buttonStyle(.glass).buttonBorderShape(.circle).padding()
                .accessibilityLabel("最新のメッセージ")
                .accessibilityIdentifier("scroll-to-latest")
            }
        }
        .onDisappear { renderCache.cancelLoads() }
        .task(id: readRetry) {
            guard parentID == nil, !readStateLoaded else { return }
            readStateError = false
            do {
                let state = try await workspace.client.readState(channelID: store.channel.id)
                try Task.checkCancellation()
                initialReadState = state
            } catch {
                guard !Task.isCancelled else { return }
                readStateError = true
                if case ZooError.expiredSession = error { workspace.report(error) }
            }
            readStateLoaded = true
        }
        .task(id: readID) {
            guard let candidate = readID else { return }
            // 初期位置決めやスクロールの途中を既読にせず、最新の投稿が見え続けた場合に確定する。
            do { try await Task.sleep(for: .milliseconds(400)) } catch { return }
            guard readID == candidate else { return }
            await store.markRead(through: candidate)
        }
        .sensoryFeedback(.impact(weight: .light), trigger: selectedMessage != nil) { _, isPresented in
            isPresented
        }
        .sheet(item: $selectedMessage, onDismiss: {
            if let id = replyAfterDismiss {
                if store.message(id: id) != nil { openThread(id) }
            } else if wasComposingBeforeMenu, isVisible, store.canPost {
                composerFocused = true
            }
            replyAfterDismiss = nil
            wasComposingBeforeMenu = false
        }) { selected in
            if let message = store.message(id: selected.id) {
                MessageActionsView(store: store, workspace: workspace, message: message,
                                   canOpenThread: canOpenThread(for: message)) {
                    replyAfterDismiss = parentID ?? message.id
                    // キーボードのフォーカスを渡す前にメニューを閉じ、画面遷移だけをアニメーションさせる。
                    var transaction = Transaction()
                    transaction.disablesAnimations = true
                    withTransaction(transaction) { selectedMessage = nil }
                }
            }
        }
        .onChange(of: selectedActionsAvailable) { _, available in
            if !available { selectedMessage = nil }
        }
    }

    private func timelineDivider(_ title: String, color: Color) -> some View {
        HStack(spacing: 10) {
            Rectangle().fill(color.opacity(0.6)).frame(height: 1)
            Text(title).font(.caption.bold()).fixedSize()
            Rectangle().fill(color.opacity(0.6)).frame(height: 1)
        }.foregroundStyle(color).padding(.horizontal, 16).padding(.vertical, 10)
            .accessibilityElement(children: .combine)
    }

    private func messageRow(_ message: Message) -> some View {
        let renderState = renderCache.state(for: message.id)
        let person = workspace.people[message.user_id]
        let name = person?.name ?? "ユーザー"
        let showActions = {
            guard hasActions(for: message) else { return }
            wasComposingBeforeMenu = composerFocused
            composerFocused = false
            selectedMessage = message
        }
        let pressingChanged: (Bool) -> Void = { pressed in
            if pressed { pressingMessageID = message.id }
            else if pressingMessageID == message.id { pressingMessageID = nil }
        }
        return HStack(alignment: .top, spacing: 12) {
            AvatarView(name: name, url: person?.avatarURL)
                .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
            VStack(alignment: .leading, spacing: 6) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(name).font(.subheadline.bold())
                        if person?.is_bot == true { Text("アプリ").font(.caption2).foregroundStyle(.secondary) }
                        if let date = message.date {
                            Text(date, format: .dateTime.hour().minute())
                                .font(.caption2).foregroundStyle(.secondary)
                        }
                        if message.updated_at != nil { Text("編集済み").font(.caption2).foregroundStyle(.secondary) }
                    }
                    .contentShape(Rectangle())
                    .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
                    if !message.content.isEmpty || store.attachments(for: message).isEmpty {
                        MessageContentView(message: message, workspace: workspace, renderState: renderState)
                        .font(.body)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityIdentifier("message-content-\(message.id)")
                        .accessibilityAction(named: "メッセージの操作", showActions)
                        .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
                    }
                    MessageAttachmentsView(attachments: store.attachments(for: message), workspace: workspace, renderState: renderState,
                                           channelID: store.channel.id, messageID: message.id, isVisible: isVisible)
                }
                // スタンプの押下を本文の長押し判定から外し、メッセージ全体を暗くしない。
                ReactionChips(store: store, workspace: workspace, message: message)
                if parentID == nil {
                    let count = store.replies(to: message.id).count
                    if count > 0 {
                        Text("\(count) 件の返信").font(.caption).foregroundStyle(.secondary)
                            .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
                    }
                }
                if let error = store.reactionError(for: message.id) {
                    Text(error).font(.caption).foregroundStyle(.red)
                }
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .background(store.focusedMessageID == message.id ? Color.accentColor.opacity(0.08) : .clear)
        .contentShape(Rectangle())
        .modifier(MessagePressFeedback(isEnabled: hasActions(for: message), isPressing: pressingMessageID == message.id))
    }
}

private struct MessagePressFeedback: ViewModifier {
    let isEnabled: Bool
    let isPressing: Bool
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content
            .overlay {
                RoundedRectangle(cornerRadius: 12)
                    .fill(.black.opacity(colorScheme == .dark ? 0.32 : 0.12))
                    .opacity(isEnabled && isPressing ? 1 : 0)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
    }
}

struct AvatarView: View {
    let name: String
    var url: URL? = nil

    var body: some View {
        AsyncImage(url: url) { phase in
            if let image = phase.image {
                image.resizable().scaledToFill()
            } else {
                Text(String(name.prefix(1))).font(.subheadline.bold()).foregroundStyle(.tint)
            }
        }
        .id(url)
        .frame(width: 38, height: 38)
        .background(.tint.opacity(0.12))
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .accessibilityHidden(true)
    }
}


struct EditMessageView: View {
    @Bindable var store: ConversationStore
    let message: Message
    @State private var content: String
    @State private var saving = false
    @Environment(\.dismiss) private var dismiss

    init(store: ConversationStore, message: Message) {
        self.store = store
        self.message = message
        _content = State(initialValue: message.content)
    }

    var body: some View {
        NavigationStack {
            Form {
                TextField("メッセージ", text: $content, axis: .vertical).lineLimit(5...15)
                    .accessibilityIdentifier("edit-message-input")
                if let error = store.actionError { Text(error).foregroundStyle(.red) }
            }
            .navigationTitle("メッセージを編集").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("キャンセル") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("保存") { saving = true }.disabled(saving || !store.validContent(content))
                        .keyboardShortcut("s", modifiers: .command)
                        .accessibilityIdentifier("save-message-edit")
                }
            }
            .task(id: saving) {
                if saving {
                    if await store.edit(message, content: content) { dismiss() }
                    saving = false
                }
            }
        }
    }
}
