import SwiftUI

struct ConversationView: View {
    @Bindable var store: ConversationStore
    let workspace: WorkspaceStore
    @Environment(\.scenePhase) private var scenePhase
    @State private var thread: ThreadTarget?
    @State private var threadFocusRequest: UUID?
    @State private var retry = UUID()

    private struct ThreadTarget: Identifiable { let id: String }

    var body: some View {
        VStack(spacing: 0) {
            if let description = store.channel.description, !description.isEmpty {
                Text(description).font(.caption).foregroundStyle(.secondary)
                    .lineLimit(2).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal).padding(.bottom, 8)
            }
            if store.channel.encrypted == true {
                ContentUnavailableView("暗号化された会話", systemImage: "lock.shield",
                                       description: Text("この iOS 版では暗号化された会話をまだ開けません。対応済みの Zoo クライアントをご利用ください。"))
            } else {
                if let error = store.syncError ?? store.reactionSyncError {
                    HStack {
                        Label(error, systemImage: "wifi.exclamationmark").font(.caption)
                        Spacer()
                        Button("再試行") {
                            store.retryAccess()
                            retry = UUID()
                        }.font(.caption.bold())
                    }.padding(12).background(.orange.opacity(0.12))
                }
                MessageList(store: store, workspace: workspace, parentID: nil,
                            canMarkRead: thread == nil && store.focusedMessageID == nil) { id in
                    threadFocusRequest = store.canPost ? UUID() : nil
                    thread = ThreadTarget(id: id)
                }
            }
        }
        .background(Color(.systemBackground))
        .navigationTitle(workspace.name(for: store.channel))
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom, spacing: 0) { ComposerView(store: store, parentID: nil) }
        .task(id: "\(scenePhase)-\(retry)-\(store.channel.encrypted == true)-\(store.blocked)") {
            if scenePhase == .active, !store.blocked { await store.run() }
        }
        .task(id: "\(scenePhase)-\(retry)-\(store.channel.encrypted == true)-\(store.blocked)-\(store.needsReactionStream)") {
            if scenePhase == .active, store.channel.encrypted != true, !store.blocked, store.needsReactionStream {
                await store.runReactions()
            }
        }
        .task(id: scenePhase == .active ? store.reactionResyncTicket : nil) {
            guard scenePhase == .active, store.reactionResyncTicket != nil else { return }
            do { try await Task.sleep(for: .seconds(15)) } catch { return }
            retry = UUID()
        }
        .task(id: scenePhase) {
            if scenePhase == .active {
                async let emojis: Void = workspace.loadEmojis()
                async let accounts: Void = workspace.loadEmojiAccounts()
                _ = await (emojis, accounts)
            }
        }
        .task(id: "\(scenePhase)-\(Set(store.messages.map(\.user_id)).sorted())") {
            if scenePhase == .active { await workspace.resolveAuthors(in: store.messages) }
        }
        .onChange(of: store.focusedMessageID, initial: true) { _, id in openFocusedThread(id) }
        .onChange(of: store.messages.map(\.id)) { _, _ in openFocusedThread(store.focusedMessageID) }
        .sheet(item: $thread, onDismiss: { threadFocusRequest = nil }) { target in
            NavigationStack {
                MessageList(store: store, workspace: workspace, parentID: target.id, canMarkRead: false) { _ in
                    threadFocusRequest = UUID()
                }
                    .navigationTitle("スレッド").navigationBarTitleDisplayMode(.inline)
                    .safeAreaInset(edge: .bottom, spacing: 0) {
                        ComposerView(store: store, parentID: target.id, focusRequest: threadFocusRequest)
                    }
                    .toolbar { ToolbarItem(placement: .confirmationAction) { Button("閉じる") { thread = nil } } }
            }
        }
    }

    private func openFocusedThread(_ id: String?) {
        guard let id, let message = store.messages.first(where: { $0.id == id }) else { return }
        if let parent = message.parent_id, store.messages.contains(where: { $0.id == parent }) {
            thread = ThreadTarget(id: parent)
        } else { thread = nil }
    }
}

private struct MessageList: View {
    @Bindable var store: ConversationStore
    let workspace: WorkspaceStore
    let parentID: String?
    let canMarkRead: Bool
    let openThread: (String) -> Void
    @Environment(\.scenePhase) private var scenePhase
    @State private var followingLatest = false
    @State private var positionedInitialMessages = false
    @State private var selectedMessage: Message?
    @State private var replyAfterDismiss: String?

    private var messages: [Message] {
        if let parentID { return store.messages.filter { $0.id == parentID || $0.parent_id == parentID } }
        return store.roots
    }

    private var readID: String? {
        guard canMarkRead, parentID == nil, followingLatest, scenePhase == .active,
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
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if store.loading && messages.isEmpty {
                        ProgressView("メッセージを読み込んでいます…")
                            .frame(maxWidth: .infinity).padding(40)
                    } else if messages.isEmpty && store.syncError == nil {
                        ContentUnavailableView("会話を始めましょう", systemImage: "bubble.left.and.bubble.right",
                                               description: Text("最初のメッセージを送ってみませんか。"))
                    }
                    if let error = store.actionError {
                        Text(error).font(.callout).foregroundStyle(.red).padding()
                    }
                    ForEach(messages) { message in
                        messageRow(message)
                            .id(message.id)
                    }
                    Color.clear.frame(height: 1).id("conversation-bottom")
                }.padding(.vertical, 8)
            }
            .accessibilityIdentifier(parentID == nil ? "message-list" : "thread-message-list")
            .defaultScrollAnchor(.bottom, for: .initialOffset)
            .scrollDismissesKeyboard(.interactively)
            .onScrollGeometryChange(for: Bool.self) { geometry in
                geometry.contentOffset.y + geometry.containerSize.height >= geometry.contentSize.height - 80
            } action: { _, atBottom in followingLatest = atBottom }
            .onChange(of: messages.last?.id, initial: true) { _, lastID in
                guard lastID != nil else { return }
                if let focused = store.focusedMessageID, messages.contains(where: { $0.id == focused }) {
                    positionedInitialMessages = true
                    proxy.scrollTo(focused, anchor: .center)
                    store.focusedMessageID = nil
                } else if !positionedInitialMessages || followingLatest {
                    positionedInitialMessages = true
                    proxy.scrollTo("conversation-bottom", anchor: .bottom)
                }
            }
            .onChange(of: store.focusedMessageID, initial: true) { _, id in
                if let id, messages.contains(where: { $0.id == id }) {
                    positionedInitialMessages = true
                    proxy.scrollTo(id, anchor: .center)
                    store.focusedMessageID = nil
                }
            }
            .overlay(alignment: .bottomTrailing) {
                if !followingLatest, !messages.isEmpty {
                    Button("最新のメッセージ", systemImage: "arrow.down") {
                        store.focusedMessageID = nil
                        withAnimation { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
                    }
                    .labelStyle(.iconOnly).padding(12).glassEffect().padding()
                }
            }
        }
        .task(id: readID) { if let readID { await store.markRead(through: readID) } }
        .sensoryFeedback(.impact(weight: .light), trigger: selectedMessage != nil) { _, isPresented in
            isPresented
        }
        .sheet(item: $selectedMessage, onDismiss: {
            if let id = replyAfterDismiss, store.message(id: id) != nil { openThread(id) }
            replyAfterDismiss = nil
        }) { selected in
            if let message = store.message(id: selected.id) {
                MessageActionsView(store: store, workspace: workspace, message: message,
                                   canOpenThread: canOpenThread(for: message)) {
                    replyAfterDismiss = parentID ?? message.id
                }
            }
        }
        .onChange(of: selectedActionsAvailable) { _, available in
            if !available { selectedMessage = nil }
        }
    }

    private func messageRow(_ message: Message) -> some View {
        let person = workspace.people[message.user_id]
        let name = person?.name ?? "ユーザー"
        let showActions = {
            if hasActions(for: message) { selectedMessage = message }
        }
        return HStack(alignment: .top, spacing: 12) {
            AvatarView(name: name, url: person?.avatarURL)
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(name).font(.subheadline.bold())
                    if person?.is_bot == true { Text("アプリ").font(.caption2).foregroundStyle(.secondary) }
                    if let date = message.date {
                        Text(date, format: .dateTime.month().day().hour().minute())
                            .font(.caption2).foregroundStyle(.secondary)
                    }
                    if message.updated_at != nil { Text("編集済み").font(.caption2).foregroundStyle(.secondary) }
                }
                Text(message.content.isEmpty ? "添付ファイルまたはアプリのメッセージ" : message.content)
                    .font(.body)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("message-content-\(message.id)")
                    .accessibilityAction(named: "メッセージの操作", showActions)
                ReactionChips(store: store, workspace: workspace, message: message)
                if parentID == nil {
                    let count = store.replies(to: message.id).count
                    if count > 0 {
                        Text("\(count) 件の返信").font(.caption).foregroundStyle(.secondary)
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
        .modifier(MessagePressFeedback(isEnabled: hasActions(for: message), showActions: showActions))
    }
}

private struct MessagePressFeedback: ViewModifier {
    let isEnabled: Bool
    let showActions: () -> Void
    @State private var isPressing = false
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
            .onLongPressGesture(perform: showActions, onPressingChanged: { isPressing = $0 })
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

private struct ComposerView: View {
    @Bindable var store: ConversationStore
    let parentID: String?
    var focusRequest: UUID?
    @State private var submitting = false
    @FocusState private var focused: Bool

    private var key: String { store.draftKey(parentID) }
    private var draft: Binding<String> {
        Binding(get: { store.drafts[key] ?? "" }, set: { store.drafts[key] = $0 })
    }

    var body: some View {
        VStack(spacing: 8) {
            if let error = store.sendErrors[key] {
                Text(error).font(.caption).foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading).accessibilityIdentifier("send-error")
            }
            if store.canPost {
                HStack(alignment: .bottom, spacing: 10) {
                    TextField(parentID == nil ? "メッセージを入力" : "返信を入力", text: draft, axis: .vertical)
                        .lineLimit(1...6).focused($focused)
                        .padding(.horizontal, 14).padding(.vertical, 12)
                        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 20))
                        .accessibilityIdentifier(parentID == nil ? "message-input" : "reply-input")
                    Button {
                        submitting = true
                    } label: {
                        if store.sending.contains(key) { ProgressView().frame(width: 24, height: 24) }
                        else { Image(systemName: "arrow.up").font(.headline).frame(width: 24, height: 24) }
                    }
                    .buttonStyle(.borderedProminent).buttonBorderShape(.circle)
                    .disabled(submitting || store.sending.contains(key) || !store.validContent(draft.wrappedValue))
                    .accessibilityLabel("送信").accessibilityIdentifier(parentID == nil ? "send-message" : "send-reply")
                }
                if draft.wrappedValue.unicodeScalars.count > 40_000 {
                    Text("メッセージは 40,000 文字以内で入力してください。").font(.caption).foregroundStyle(.red)
                }
            } else {
                Label(store.channel.encrypted == true ? "暗号化された会話には送信できません" :
                      store.blocked ? "このチャンネルには送信できません" : "アーカイブ済みのチャンネルです", systemImage: "lock")
                    .font(.callout).foregroundStyle(.secondary).padding(.vertical, 8)
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 10)
        .background(.bar)
        .onChange(of: focusRequest, initial: true) { _, request in
            if request != nil, store.canPost { focused = true }
        }
        .task(id: submitting) {
            if submitting { await store.send(parentID: parentID); submitting = false }
        }
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
