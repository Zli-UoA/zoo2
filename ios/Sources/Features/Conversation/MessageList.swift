import SwiftUI

private struct ThreadScrollLayout: Equatable {
    let contentHeight: CGFloat
    let viewportHeight: CGFloat
    let followingLatest: Bool
    var fits: Bool { contentHeight > 0 && contentHeight <= viewportHeight }
    let topInset: CGFloat
    let bottomInset: CGFloat
}

@MainActor
private final class MessageViewport {
    var bounds = CGRect.zero
    var frames: [String: CGRect] = [:]
    var visibleIDs: Set<String> = []
    var pendingRevision: Int?
    var pendingAnchor: (id: String, point: UnitPoint)?

    func anchor(in ids: [String], excluding parentID: String?) -> (id: String, point: UnitPoint)? {
        for id in ids where id != parentID && visibleIDs.contains(id) {
            guard let frame = frames[id], frame.intersects(bounds) else { continue }
            let available = bounds.height - frame.height
            guard abs(available) > 1 else { continue }
            return (id, UnitPoint(x: 0, y: (frame.minY - bounds.minY) / available))
        }
        return nil
    }
}

private enum MessageTimelineItem: Identifiable {
    case message(Message)
    case divider(id: String, title: String, unread: Bool)
    /// 暗号化に切り替える前と後の境目（#367。Web の「ここから暗号化されています」）。
    case encryptionDivider

    var id: String {
        switch self {
        case .message(let message): message.id
        case .divider(let id, _, _): id
        case .encryptionDivider: "encryption-divider"
        }
    }
}

struct MessageList: View {
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
    @State private var pressingMessageID: String?
    @State private var initialReadState: ChannelReadState?
    @State private var readStateLoaded = false
    @State private var readStateError = false
    @State private var readRetry = 0
    @State private var appliedReadRequest: ReadRequest?
    @State private var renderCache = MessageRenderCache()
    @State private var scrollPosition = ScrollPosition(idType: String.self)
    @State private var contentFits = false
    @State private var followsLatestIntent = true
    @State private var preservesFocusedPosition = false
    @State private var userScrolling = false
    @State private var viewport = MessageViewport()

    private var positionKey: String {
        "\(messages.last?.id ?? "")|\(readStateLoaded)|\(store.loading)|\(scrollTarget.map { id in messages.contains { $0.id == id } } ?? false)"
    }

    private var messages: [Message] {
        if let parentID {
            return [store.message(id: parentID)].compactMap { $0 } + store.replies(to: parentID)
        }
        return store.roots
    }

    private var scrollTarget: String? { isVisible ? store.focusedMessageID : nil }

    private var selectedMessage: Message? {
        guard case .messageActions(let sheet) = workspace.presentedSheet,
              sheet.store === store, sheet.parentID == parentID else { return nil }
        return sheet.message
    }

    private var readID: String? {
        guard isVisible, !userScrolling, parentID == nil, !store.showingCachedMessages, readStateLoaded, initialReadState != nil, positionedInitialMessages, store.focusedMessageID == nil, followingLatest, scenePhase == .active,
              selectedMessage == nil else { return nil }
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

    private func updateHistoryAnchor() {
        // 取得結果の描画通知が先に届いても、追加前の位置を上書きしない。
        guard viewport.pendingRevision == store.historyRevision else { return }
        viewport.pendingAnchor = viewport.anchor(in: messages.map(\.id), excluding: parentID)
    }

    private func timeline(_ messages: [Message], firstUnreadID: String?) -> [MessageTimelineItem] {
        var items: [MessageTimelineItem] = []
        // 読み込んだうちで最初の暗号化したメッセージ（暗号文か `epoch` のある行）。それより上に行があるときだけ、境目を出す。
        let firstEncrypted = MessageTimeline.encryptionStartIndex(in: messages, channelEncrypted: store.channel.encrypted == true)
        for (index, message) in messages.enumerated() {
            if message.id != parentID,
               MessageTimeline.startsDay(message, after: index > 0 ? messages[index - 1] : nil), let date = message.date {
                items.append(.divider(id: "date-divider-" + message.id,
                                      title: date.formatted(.dateTime.year().month().day().weekday().locale(Locale(identifier: "ja_JP"))), unread: false))
            }
            if index == firstEncrypted { items.append(.encryptionDivider) }
            if firstUnreadID == message.id { items.append(.divider(id: "unread-divider", title: "ここから未読", unread: true)) }
            items.append(.message(message))
        }
        return items
    }

    var body: some View {
        let messages = self.messages
        let messageIDs = messages.map(\.id)
        let firstUnreadID = parentID == nil
            ? MessageTimeline.firstUnread(in: messages, state: initialReadState, userID: store.userID) : nil
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                if store.showingCachedMessages {
                    Label("保存済みのメッセージを表示しています", systemImage: "internaldrive")
                        .font(.caption).foregroundStyle(.secondary).padding(8)
                }
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
                ForEach(timeline(messages, firstUnreadID: firstUnreadID)) { item in
                    switch item {
                    case .divider(let id, let title, let unread):
                        timelineDivider(title, color: unread ? .red : .primary).accessibilityIdentifier(id)
                    case .encryptionDivider:
                        timelineDivider(E2eeText.encryptionStarts, color: .secondary, symbol: "lock.fill")
                            .accessibilityIdentifier("encryption-divider")
                    case .message(let message):
                        VStack(spacing: 0) {
                            messageRow(message)
                            if message.id == parentID { threadRepliesDivider }
                        }
                        .padding(.bottom, message.id == messages.last?.id ? 8 : 0)
                        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame in
                            viewport.frames[message.id] = frame
                            updateHistoryAnchor()
                        }
                        .id(message.id)
                    }
                }
            }
            .scrollTargetLayout()
            .padding(.top, 8)
        }
        .accessibilityIdentifier(parentID == nil ? "message-list" : "thread-message-list")
        .accessibilityValue("\(messages.count)件のメッセージ")
        .scrollBounceBehavior(store.hasOlder(parentID: parentID) ? .always : .basedOnSize)
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .defaultScrollAnchor(parentID == nil ? .bottom : .top, for: .alignment)
        .scrollPosition($scrollPosition)
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: {
            // このフレームはすでにナビゲーションと入力欄の安全領域を除いている。
            viewport.bounds = $0
            updateHistoryAnchor()
        }
        .scrollDismissesKeyboard(.interactively)
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidChangeFrameNotification)) { _ in
            guard parentID == nil, isVisible, contentFits, !userScrolling, !preservesFocusedPosition, !messages.isEmpty else { return }
            // キーボードの遷移中は安全領域が未確定なので、収まる会話は表示完了後にも末尾へ揃える。
            scrollPosition.scrollTo(edge: .bottom)
        }
        .onScrollPhaseChange { _, phase in
            if phase == .tracking || phase == .interacting {
                userScrolling = true
                followsLatestIntent = false
            } else if phase == .idle, userScrolling {
                userScrolling = false
                followsLatestIntent = followingLatest
                if followingLatest { preservesFocusedPosition = false }
                updateHistoryAnchor()
            }
        }
        .onScrollGeometryChange(for: ThreadScrollLayout.self) { geometry in
            let state = MessageScrollState(contentHeight: geometry.contentSize.height, offsetY: geometry.contentOffset.y,
                                           topInset: geometry.contentInsets.top, viewportHeight: geometry.containerSize.height)
            return ThreadScrollLayout(contentHeight: geometry.contentSize.height, viewportHeight: geometry.containerSize.height,
                                      followingLatest: state.followingLatest,
                                      topInset: geometry.contentInsets.top, bottomInset: geometry.contentInsets.bottom)
        } action: { previous, layout in
            contentFits = layout.fits
            guard isVisible, !store.loading, !userScrolling, !preservesFocusedPosition,
                  previous.contentHeight != layout.contentHeight || previous.viewportHeight != layout.viewportHeight ||
                  previous.topInset != layout.topInset || previous.bottomInset != layout.bottomInset else { return }
            if parentID == nil {
                // 入力欄が広がっても最新投稿が隠れないよう、短い会話と末尾を表示中の会話を表示領域に合わせる。
                if layout.fits, !messages.isEmpty {
                    scrollPosition.scrollTo(edge: .bottom)
                } else if previous.fits || previous.followingLatest || followsLatestIntent, let latest = messages.last {
                    scrollPosition.scrollTo(id: latest.id, anchor: .bottom)
                }
                return
            }
            if layout.fits {
                followsLatestIntent = true
                // 全文が収まるスレッドでは、末尾への位置合わせで親投稿の上に余白を残さない。
                scrollPosition.scrollTo(edge: .top)
            } else if previous.fits || followsLatestIntent, let latest = messages.last {
                // 入力欄の安全領域と遅延計測が確定しても、最新の行を基準に位置を合わせる。
                scrollPosition.scrollTo(id: latest.id, anchor: .bottom)
            }
        }
        .onScrollTargetVisibilityChange(idType: String.self, threshold: 0.01) { ids in
            viewport.visibleIDs = Set(ids)
            viewport.frames = viewport.frames.filter { viewport.visibleIDs.contains($0.key) }
            updateHistoryAnchor()
            renderCache.retain(visibleIDs: ids, orderedIDs: messageIDs)
        }
        .onScrollGeometryChange(for: CGFloat.self) { geometry in
            geometry.contentOffset.y + geometry.contentInsets.top
        } action: { previous, offset in
            guard isVisible, positionedInitialMessages, userScrolling, offset < previous else { return }
            // スレッドでは親投稿が長くても、最古の返信に近づいた時点で続きを取得する。
            let nearOldest = messages.lazy.filter { $0.id != parentID }.prefix(3)
                .contains { viewport.visibleIDs.contains($0.id) }
            if (offset < 240 || nearOldest), !store.loadingOlder {
                store.loadOlder(parentID: parentID)
                if store.loadingOlder {
                    viewport.pendingRevision = store.historyRevision
                    updateHistoryAnchor()
                }
            }
        }
        .onChange(of: messageIDs) { _, ids in renderCache.updateMessages(ids) }
        .onChange(of: store.historyRevision) { _, revision in
            guard let pending = viewport.pendingRevision, pending != revision else { return }
            if isVisible, let anchor = viewport.pendingAnchor, messageIDs.contains(anchor.id) {
                // 最古投稿が同時に削除されても、読んでいた投稿があれば同じ位置へ戻す。
                scrollPosition.scrollTo(id: anchor.id, anchor: anchor.point)
            }
            viewport.pendingRevision = nil
            viewport.pendingAnchor = nil
        }
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
                let followsLatest = store.followsFocusedMessageToLatest
                preservesFocusedPosition = !followsLatest
                followsLatestIntent = followsLatest
                positionedInitialMessages = true
                scrollPosition.scrollTo(id: focused, anchor: followsLatest ? .bottom : .center)
                store.focusedMessageID = nil
            } else if !preservesFocusedPosition && (!positionedInitialMessages || followingLatest) {
                followsLatestIntent = true
                let openingReplyComposer = !positionedInitialMessages && focusComposerOnAppear
                positionedInitialMessages = true
                if parentID == nil && contentFits {
                    // 短い履歴は下揃えの余白があるため、行の座標を使わず画面端へ合わせる。
                    scrollPosition.scrollTo(edge: .bottom)
                } else if parentID != nil && contentFits && !openingReplyComposer {
                    scrollPosition.scrollTo(edge: .top)
                } else if let last = messages.last {
                    // キーボードを開く遷移中は、実在する行を基準にして空白へスクロールしない。
                    scrollPosition.scrollTo(id: last.id, anchor: .bottom)
                }
            }
        }
        .onChange(of: scrollTarget, initial: true) { _, id in
            if id != nil { viewport.pendingRevision = nil; viewport.pendingAnchor = nil }
            if let id, id == scrollTarget, messages.contains(where: { $0.id == id }) {
                let followsLatest = store.followsFocusedMessageToLatest
                preservesFocusedPosition = !followsLatest
                followsLatestIntent = followsLatest
                positionedInitialMessages = true
                scrollPosition.scrollTo(id: id, anchor: followsLatest ? .bottom : .center)
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
                    viewport.pendingRevision = nil
                    viewport.pendingAnchor = nil
                    followsLatestIntent = true
                    preservesFocusedPosition = false
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
        .onDisappear {
            renderCache.cancelLoads()
            viewport.pendingRevision = nil
            viewport.pendingAnchor = nil
        }
        .overlay(alignment: .top) {
            if isVisible, store.loadingOlder {
                ProgressView("以前のメッセージを読み込んでいます…")
                    .font(.caption).padding(8)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                    .allowsHitTesting(false)
                    .accessibilityIdentifier("loading-older-messages")
            }
        }
        .overlay(alignment: .top) {
            if parentID == nil, let error = store.readError {
                Text(error).font(.caption).padding(8)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                    .padding(.horizontal)
                    .accessibilityIdentifier("read-save-error")
            }
        }
        .task(id: ReadRequest(retry: readRetry, generation: store.accessGeneration, blocked: store.blocked)) {
            guard parentID == nil else { return }
            let epoch = store.accessGeneration
            let request = ReadRequest(retry: readRetry, generation: epoch, blocked: store.blocked)
            guard appliedReadRequest != request || !readStateLoaded else { return }
            appliedReadRequest = request
            initialReadState = nil
            readStateLoaded = false
            positionedInitialMessages = false
            readStateError = false
            guard !store.blocked else { return }
            do {
                let state = try await workspace.client.readState(channelID: store.channel.id)
                try Task.checkCancellation()
                guard epoch == store.accessGeneration else { return }
                initialReadState = state
            } catch {
                guard !Task.isCancelled, epoch == store.accessGeneration else { return }
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
        .onChange(of: selectedActionsAvailable) { _, available in
            if !available, selectedMessage != nil { workspace.presentedSheet = nil }
        }
    }

    private struct ReadRequest: Equatable {
        let retry: Int
        let generation: UUID
        let blocked: Bool
    }

    private func timelineDivider(_ title: String, color: Color, symbol: String? = nil) -> some View {
        HStack(spacing: 10) {
            Rectangle().fill(color.opacity(0.6)).frame(height: 1)
            Group {
                if let symbol { Label(title, systemImage: symbol) } else { Text(title) }
            }
            .font(.caption.bold())
                .multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                .layoutPriority(1)
            Rectangle().fill(color.opacity(0.6)).frame(height: 1)
        }.foregroundStyle(color).padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, AppDesign.spacing)
            .accessibilityElement(children: .combine)
    }

    private var threadRepliesDivider: some View {
        let count = parentID.map { store.replyCount(to: $0) } ?? 0
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
            let wasComposing = composerFocused
            composerFocused = false
            workspace.presentedSheet = .messageActions(MessageActionSheet(
                store: store, message: message, parentID: parentID,
                canOpenThread: canOpenThread(for: message), wasComposing: wasComposing
            ))
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
                    if !message.content.isEmpty || (attachments.isEmpty && message.e2eeNotice == nil) {
                        MessageContentView(message: message, workspace: workspace, renderState: renderState)
                        .font(.body)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityIdentifier("message-content-\(message.id)")
                        .accessibilityAction(named: "メッセージの操作", showActions)
                        .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
                    }
                    if let notice = message.e2eeNotice {
                        // 復号できない理由や、送信者についての注意（Web の `e2ee.undecryptable.*`、`e2ee.sender.*` と同じ文言）。
                        MessageNoticeView(notice: notice, detail: message.e2eeNoticeDetail,
                                          symbol: message.e2eeLegacy ? "exclamationmark.triangle" : message.content.isEmpty ? "lock.slash" : "exclamationmark.shield")
                            .accessibilityIdentifier("message-e2ee-notice-\(message.id)")
                    }
                    MessageAttachmentsView(attachments: attachments, workspace: workspace, renderState: renderState,
                                           channelID: store.channel.id, messageID: message.id, isVisible: isVisible,
                                           galleryOutsets: EdgeInsets(top: 0, leading: horizontalPadding + AvatarView.size + avatarSpacing,
                                                                     bottom: 0, trailing: horizontalPadding),
                                           showMessageActions: showActions, messagePressingChanged: pressingChanged)
                        .accessibilityActions {
                            if message.content.isEmpty, hasActions(for: message) {
                                Button("メッセージの操作", action: showActions)
                            }
                        }
                }
                // スタンプの押下を本文の長押し判定から外し、メッセージ全体を暗くしない。
                ReactionChips(store: store, workspace: workspace, message: message)
                if parentID == nil {
                    let replies = store.replies(to: message.id)
                    let replyCount = store.replyCount(to: message.id)
                    if replyCount > 0 {
                        ThreadReplyLink(replies: replies, replyCount: replyCount, workspace: workspace) {
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
            (store.focusedMessageID == message.id ? AppDesign.selectionBackground : .clear)
                .contentShape(Rectangle())
                .onLongPressGesture(perform: showActions, onPressingChanged: pressingChanged)
                .accessibilityHidden(true)
        }
        .contentShape(Rectangle())
        .modifier(MessagePressFeedback(isEnabled: hasActions(for: message), isPressing: pressingMessageID == message.id))
    }
}

/// 暗号化チャンネルのメッセージに添える注意（アイコンと短い文）。くわしい説明があるときは、アイコンと文の全体が 1 つの
/// ボタンになり、タップすると説明を出す（右端の ⓘ は、タップできることを示す）。
private struct MessageNoticeView: View {
    let notice: String
    let detail: String?
    let symbol: String
    @State private var showingDetail = false

    var body: some View {
        Group {
            if let detail {
                Button { showingDetail = true } label: {
                    HStack(spacing: 6) {
                        Label(notice, systemImage: symbol)
                        Image(systemName: "info.circle")
                        Spacer(minLength: 0)
                    }
                    // 文字の部分だけでなく、行の全体をタップできるようにする。
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityHint("くわしい説明を表示します")
                .accessibilityIdentifier("message-e2ee-notice-button")
                .popover(isPresented: $showingDetail) {
                    // 折り返して、全文を出す（高さを内容に合わせる）。
                    Text(detail)
                        .font(.callout).foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(16)
                        .frame(width: 320, alignment: .leading)
                        .presentationCompactAdaptation(.popover)
                }
            } else {
                Label(notice, systemImage: symbol)
            }
        }
        .font(.caption).foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
