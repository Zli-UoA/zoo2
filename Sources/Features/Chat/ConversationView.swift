import SwiftUI

struct ConversationView: View {
    @Bindable var store: ConversationStore
    let workspace: WorkspaceStore
    let route: ChatRoute
    @FocusState private var composerFocused: Bool
    @State private var showingDetails = false
    @State private var didRequestInitialFocus = false
    @State private var navigationContentWidth: CGFloat?

    private var parentID: String? {
        if case .thread(_, let parent, _) = route { return parent }
        return nil
    }

    private var isVisible: Bool { workspace.path.last == route }

    private var focusComposerOnAppear: Bool {
        if case .thread(_, _, true) = route { return true }
        return false
    }

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
                    .padding(.horizontal, AppDesign.horizontalInset).padding(.bottom, AppDesign.spacing)
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
                            focusComposerOnAppear: focusComposerOnAppear,
                            composerFocused: $composerFocused) { id in
                    if parentID == nil {
                        workspace.openThread(id, in: store, focusComposer: store.canPost)
                    } else {
                        composerFocused = store.canPost
                    }
                }
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { navigationContentWidth = $0 }
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
                        .padding(.horizontal, 16)
                        // タイトル欄自身は内容幅へ縮むため、本文の安全領域内の幅から左右のボタンと余白を引く。
                        .frame(width: navigationContentWidth.map { width in
                            max(0, width - 2 * (AppDesign.minimumControlSize + 16 + 12))
                        })
                        .frame(height: AppDesign.minimumControlSize)
                        .contentShape(Capsule())
                    }
                    .buttonStyle(.plain)
                    .glassEffect(.regular.interactive(), in: Capsule())
                    .foregroundStyle(.primary)
                    .tint(.primary)
                    .accessibilityLabel("\(workspace.name(for: store.channel))の詳細")
                    .accessibilityIdentifier("channel-details")
                }
            } else {
                ToolbarItem(placement: .principal) {
                    VStack(spacing: 2) {
                        Text("スレッド").font(.headline)
                        Text(store.channel.is_dm ? workspace.name(for: store.channel) : "#\(workspace.name(for: store.channel))")
                            .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            .accessibilityIdentifier("thread-channel-name")
                    }
                }
            }
            if parentID == nil, store.canCall, !composerFocused {
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
            if focusComposerOnAppear {
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

private struct ThreadScrollLayout: Equatable {
    let contentHeight: CGFloat
    let viewportHeight: CGFloat
    var fits: Bool { contentHeight > 0 && contentHeight <= viewportHeight }
    var bottomOffset: CGFloat { max(0, contentHeight - viewportHeight) }
}

private struct MessageList: View {
    @Bindable var store: ConversationStore
    let workspace: WorkspaceStore
    let parentID: String?
    let isVisible: Bool
    let focusComposerOnAppear: Bool
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
    @State private var contentFits = false
    @State private var threadFollowsLatest = true
    @State private var threadUserScrolling = false

    private var positionKey: String {
        "\(messages.last?.id ?? "")|\(readStateLoaded)|\(store.loading)"
    }

    private var messages: [Message] {
        if let parentID {
            return [store.message(id: parentID)].compactMap { $0 } + store.replies(to: parentID)
        }
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
        let messages = self.messages
        let messageIDs = messages.map(\.id)
        let firstUnreadID = parentID == nil
            ? MessageTimeline.firstUnread(in: messages, state: initialReadState, userID: store.userID) : nil
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                if !store.loading && messages.isEmpty && store.syncError == nil {
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
                        if message.id != parentID,
                           MessageTimeline.startsDay(message, after: index > 0 ? messages[index - 1] : nil), let date = message.date {
                            timelineDivider(date.formatted(.dateTime.year().month().day().weekday().locale(Locale(identifier: "ja_JP"))), color: .primary)
                                .accessibilityIdentifier("date-divider-\(message.id)")
                        }
                        if firstUnreadID == message.id {
                            timelineDivider("ここから未読", color: .red).accessibilityIdentifier("unread-divider")
                        }
                        messageRow(message)
                        if message.id == parentID {
                            threadRepliesDivider
                        }
                    }
                    .padding(.bottom, index == messages.count - 1 ? 8 : 0)
                    .id(message.id)
                }
            }
            .scrollTargetLayout()
            .padding(.top, 8)
        }
        .accessibilityIdentifier(parentID == nil ? "message-list" : "thread-message-list")
        .scrollBounceBehavior(.basedOnSize)
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .defaultScrollAnchor(parentID == nil ? .bottom : .top, for: .alignment)
        .scrollPosition($scrollPosition)
        .scrollDismissesKeyboard(.interactively)
        .onScrollPhaseChange { _, phase in
            guard parentID != nil else { return }
            if phase == .tracking || phase == .interacting {
                threadUserScrolling = true
                threadFollowsLatest = false
            } else if phase == .idle, threadUserScrolling {
                threadUserScrolling = false
                threadFollowsLatest = followingLatest
            }
        }
        .onScrollGeometryChange(for: ThreadScrollLayout.self) { geometry in
            ThreadScrollLayout(contentHeight: geometry.contentSize.height, viewportHeight: geometry.containerSize.height)
        } action: { previous, layout in
            contentFits = layout.fits
            guard parentID != nil, isVisible, !store.loading, !threadUserScrolling,
                  previous.contentHeight != layout.contentHeight || previous.viewportHeight != layout.viewportHeight else { return }
            if layout.fits {
                threadFollowsLatest = true
                // 全文が収まるスレッドでは、末尾への位置合わせで親投稿の上に余白を残さない。
                scrollPosition.scrollTo(edge: .top)
            } else if previous.fits || threadFollowsLatest {
                // キーボードの遷移中は末尾アンカーの余白が未確定なので、実際の表示領域から位置を求める。
                scrollPosition.scrollTo(y: layout.bottomOffset)
            }
        }
        .onScrollTargetVisibilityChange(idType: String.self, threshold: 0.01) { ids in
            renderCache.retain(visibleIDs: ids, orderedIDs: messageIDs)
        }
        .onChange(of: messageIDs) { _, ids in renderCache.updateMessages(ids) }
        .onScrollGeometryChange(for: MessageScrollState.self) { geometry in
            MessageScrollState(contentHeight: geometry.contentSize.height, offsetY: geometry.contentOffset.y,
                               topInset: geometry.contentInsets.top, viewportHeight: geometry.containerSize.height)
        } action: { _, state in
            // 画面遷移中の初回通知も保持し、移動先に応じて最新ボタンを表示する。
            followingLatest = state.followingLatest
            showsLatestButton = state.showsLatestButton
        }
        .onChange(of: positionKey, initial: true) { _, _ in
            guard isVisible, !messages.isEmpty, !store.loading, parentID != nil || readStateLoaded else { return }
            if let focused = scrollTarget, messages.contains(where: { $0.id == focused }) {
                threadFollowsLatest = false
                positionedInitialMessages = true
                scrollPosition.scrollTo(id: focused, anchor: .center)
                store.focusedMessageID = nil
            } else if !positionedInitialMessages || followingLatest {
                threadFollowsLatest = true
                let openingReplyComposer = !positionedInitialMessages && focusComposerOnAppear
                positionedInitialMessages = true
                if parentID == nil && contentFits {
                    // 全文が収まる場合は行への位置合わせで過剰に移動せず、入力欄との重なりを防ぐ。
                    scrollPosition.scrollTo(edge: .bottom)
                } else if parentID != nil && contentFits && !openingReplyComposer {
                    scrollPosition.scrollTo(edge: .top)
                } else if parentID != nil && !openingReplyComposer {
                    scrollPosition.scrollTo(edge: .bottom)
                } else if let last = messages.last {
                    // キーボードを開く遷移中は、実在する行を基準にして空白へスクロールしない。
                    scrollPosition.scrollTo(id: last.id, anchor: .bottom)
                }
            }
        }
        .onChange(of: scrollTarget, initial: true) { _, id in
            if let id, messages.contains(where: { $0.id == id }) {
                threadFollowsLatest = false
                positionedInitialMessages = true
                scrollPosition.scrollTo(id: id, anchor: .center)
                store.focusedMessageID = nil
            }
        }
        .overlay(alignment: .center) {
            if store.loading && messages.isEmpty {
                ProgressView("メッセージを読み込んでいます…")
                    .padding(40)
            }
        }
        .overlay(alignment: .bottomTrailing) {
            if positionedInitialMessages, showsLatestButton, !messages.isEmpty {
                Button {
                    guard let last = messages.last else { return }
                    threadFollowsLatest = true
                    store.focusedMessageID = nil
                    // 長文の遅延計測中に行の高さと位置がアニメーションして重なるのを避ける。
                    scrollPosition.scrollTo(id: last.id, anchor: .bottom)
                } label: {
                    Image(systemName: "arrow.down").frame(width: 44, height: 44)
                }
                .buttonStyle(.glass).buttonBorderShape(.circle).padding()
                .accessibilityLabel("最新のメッセージ")
                .accessibilityIdentifier("scroll-to-latest")
            }
        }
        .onDisappear { renderCache.cancelLoads() }
        .overlay(alignment: .top) {
            if parentID == nil, let error = store.readError {
                Text(error).font(.caption).padding(8)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                    .padding(.horizontal)
                    .accessibilityIdentifier("read-save-error")
            }
        }
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
            Text(title).font(.caption.bold())
                .multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                .layoutPriority(1)
            Rectangle().fill(color.opacity(0.6)).frame(height: 1)
        }.foregroundStyle(color).padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, AppDesign.spacing)
            .accessibilityElement(children: .combine)
    }

    private var threadRepliesDivider: some View {
        let count = parentID.map { store.replies(to: $0).count } ?? 0
        return VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 12) {
                Text("\(count) 件の返信")
                    .font(.caption.weight(.medium)).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .layoutPriority(1)
                    .accessibilityIdentifier("thread-reply-count")
                Rectangle().fill(Color(.separator)).frame(height: 0.5)
                    .accessibilityHidden(true)
            }
            if count == 0, !store.loading, store.syncError == nil {
                Text(store.canPost ? "まだ返信はありません。最初の返信を送ってみましょう。" : "まだ返信はありません。")
                    .font(.callout).foregroundStyle(.secondary)
                    .accessibilityIdentifier("thread-empty-state")
            }
        }
        .padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, AppDesign.spacing)
    }

    private func messageRow(_ message: Message) -> some View {
        let horizontalPadding = AppDesign.horizontalInset
        let avatarSpacing = AppDesign.spacing
        let attachments = store.attachments(for: message)
        let renderState = renderCache.state(for: message.id)
        let hasFullWidthImages = attachments.filter(\.isImage).count > 1 || attachments.contains {
            $0.isImage && $0.is_sensitive == true
        }
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
        return HStack(alignment: .top, spacing: avatarSpacing) {
            AvatarView(name: name, url: person?.avatarURL)
                .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
            VStack(alignment: .leading, spacing: 6) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(name).font(.subheadline.bold())
                        if person?.is_bot == true { Text("アプリ").font(.caption2).foregroundStyle(.secondary) }
                        if let date = message.date {
                            Text(date, format: message.id == parentID ? .dateTime.month().day().hour().minute() : .dateTime.hour().minute())
                                .font(.caption2).foregroundStyle(.secondary)
                        }
                        if message.updated_at != nil { Text("編集済み").font(.caption2).foregroundStyle(.secondary) }
                    }
                    .frame(minHeight: message.content.isEmpty && hasFullWidthImages ? AvatarView.size : nil, alignment: .topLeading)
                    .contentShape(Rectangle())
                    .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
                    if !message.content.isEmpty || attachments.isEmpty {
                        MessageContentView(message: message, workspace: workspace, renderState: renderState)
                        .font(.body)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityIdentifier("message-content-\(message.id)")
                        .accessibilityAction(named: "メッセージの操作", showActions)
                        .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
                    }
                    MessageAttachmentsView(attachments: attachments, workspace: workspace, renderState: renderState,
                                           channelID: store.channel.id, messageID: message.id, isVisible: isVisible,
                                           galleryOutsets: EdgeInsets(top: 0, leading: horizontalPadding + AvatarView.size + avatarSpacing,
                                                                     bottom: 0, trailing: horizontalPadding),
                                           showMessageActions: showActions, messagePressingChanged: pressingChanged)
                }
                // スタンプの押下を本文の長押し判定から外し、メッセージ全体を暗くしない。
                ReactionChips(store: store, workspace: workspace, message: message)
                if parentID == nil {
                    let replies = store.replies(to: message.id)
                    if !replies.isEmpty {
                        ThreadReplyLink(replies: replies, workspace: workspace) {
                            composerFocused = false
                            workspace.openThread(message.id, in: store, focusComposer: false)
                        }
                        .accessibilityIdentifier("thread-replies-\(message.id)")
                    }
                }
                if let error = store.reactionError(for: message.id) {
                    Text(error).font(.caption).foregroundStyle(.red)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, horizontalPadding).padding(.vertical, AppDesign.spacing)
        .background {
            // 余白でだけ長押しを受け取り、添付やスタンプ固有の操作を優先する。
            (store.focusedMessageID == message.id ? AppDesign.tint.opacity(0.08) : .clear)
                .contentShape(Rectangle())
                .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
                .accessibilityHidden(true)
        }
        .contentShape(Rectangle())
        .modifier(MessagePressFeedback(isEnabled: hasActions(for: message), isPressing: pressingMessageID == message.id))
    }
}

private struct ThreadReplyLink: View {
    let replies: [Message]
    let workspace: WorkspaceStore
    let open: () -> Void
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    private var participantIDs: [String] {
        var seen = Set<String>()
        return replies.map(\.user_id).filter { seen.insert($0).inserted }
    }

    private var latestReply: String? {
        replies.last?.date.map { "最終返信 \($0.formatted(.dateTime.month().day().hour().minute().locale(Locale(identifier: "ja_JP"))))" }
    }

    var body: some View {
        let participants = participantIDs
        let participantNames = participants.map { workspace.people[$0]?.name ?? "ユーザー" }.joined(separator: "、")
        let details = ["返信したメンバー: \(participantNames)", latestReply].compactMap { $0 }.joined(separator: "。")
        let layout = dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
            : AnyLayout(HStackLayout(spacing: 8))
        Button(action: open) {
            HStack(spacing: 8) {
                layout {
                    HStack(spacing: 3) {
                        ForEach(participants.prefix(3), id: \.self) { id in
                            let person = workspace.people[id]
                            AvatarView(name: person?.name ?? "ユーザー", url: person?.avatarURL, dimension: 24)
                        }
                    }
                    VStack(alignment: .leading, spacing: 3) {
                        Text("\(replies.count) 件の返信").font(.caption.weight(.semibold)).foregroundStyle(.tint)
                        if let latestReply {
                            Text(latestReply).font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                Image(systemName: "chevron.right").font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 8).padding(.vertical, 6)
            .frame(minHeight: 44)
            .background(.tint.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .tint(Color(.link))
        .accessibilityLabel("\(replies.count) 件の返信")
        .accessibilityValue(details)
        .accessibilityHint("スレッドを開きます")
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
    static let size: CGFloat = 38
    let name: String
    var url: URL? = nil
    var dimension: CGFloat = Self.size

    var body: some View {
        AsyncImage(url: url) { phase in
            if let image = phase.image {
                image.resizable().scaledToFill()
            } else {
                Text(String(name.prefix(1)))
                    .font(.system(size: dimension * 0.4, weight: .semibold))
                    .lineLimit(1).minimumScaleFactor(0.25).foregroundStyle(.secondary)
            }
        }
        .id(url)
        .frame(width: dimension, height: dimension)
        .background(Color(.secondarySystemBackground))
        .clipShape(Circle())
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
            AppForm {
                TextField("メッセージ", text: $content, axis: .vertical).lineLimit(5...15)
                    .accessibilityIdentifier("edit-message-input")
                if let error = store.actionError { Text(error).foregroundStyle(.red) }
            }
            .navigationTitle("メッセージを編集").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } }
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
