import SwiftUI

struct InboxView: View {
    let store: InboxStore
    let workspace: WorkspaceStore
    let isActive: Bool
    let openChat: (String, String?) async -> Bool
    @Environment(\.scenePhase) private var scenePhase
    @State private var readingAll = false

    var body: some View {
        List {
            Group {
                if let error = store.error {
                    Label(error, systemImage: "exclamationmark.circle")
                        .foregroundStyle(.secondary)
                        .font(.subheadline)
                        .padding(AppDesign.horizontalInset)
                }
                if store.entries.isEmpty {
                    if store.loading {
                        ProgressView("通知を読み込み中…").frame(maxWidth: .infinity).padding(.vertical, 48)
                    } else if store.error == nil {
                        ContentUnavailableView("通知はありません", systemImage: "bell")
                    }
                }
                ForEach(store.entries) { entry in
                    VStack(spacing: 0) {
                        InboxCard(entry: entry, store: store, workspace: workspace,
                                  isActive: isActive && scenePhase == .active, openChat: openChat)
                        Divider().padding(.leading, AppDesign.horizontalInset)
                    }
                }
                if store.hasMore {
                    Button(store.loadingMore ? "読み込み中…" : "さらに通知を表示") {
                        Task { await store.loadMore() }
                    }
                    .frame(maxWidth: .infinity)
                    .frame(minHeight: AppDesign.minimumControlSize)
                    .disabled(store.loadingMore)
                    .accessibilityIdentifier("inbox-load-more")
                }
            }
            .listRowInsets(EdgeInsets())
            .listRowSeparator(.hidden)
            .listRowBackground(AppDesign.background)
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .contentMargins(.top, AppDesign.spacing, for: .scrollContent)
        .background(AppDesign.background.ignoresSafeArea())
        .navigationTitle("通知")
        .navigationBarTitleDisplayMode(.large)
        .accessibilityIdentifier("inbox-list")
        .refreshable { await store.refresh() }
        .toolbar {
            if store.unreadCount > 0 {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("すべて既読", systemImage: "checkmark.circle") {
                        readingAll = true
                        Task {
                            await store.markAllRead()
                            readingAll = false
                        }
                    }
                    .accessibilityLabel("すべて既読にする")
                    .accessibilityIdentifier("inbox-read-all")
                    .disabled(readingAll)
                }
            }
        }
    }
}

private struct InboxCard: View {
    let entry: InboxEntry
    let store: InboxStore
    let workspace: WorkspaceStore
    let isActive: Bool
    let openChat: (String, String?) async -> Bool
    @State private var visible = false
    @State private var fetchedPreview: FetchedPreview?
    @State private var openFailed = false
    @State private var opening = false
    @State private var responding = false
    @State private var retry = 0

    private var channel: Channel? { workspace.channels.first { $0.id == entry.channelID } }
    private var hasChannel: Bool { channel != nil || entry.channelName != nil }
    private var missedCall: Bool { entry.kind == "missed_call" }
    private var person: Person? { entry.actorIDs.first.flatMap { workspace.people[$0] } }
    private var actor: String { entry.actorName ?? person?.name ?? "ユーザー" }
    private var actorSummary: String {
        entry.actorIDs.count > 1 ? "\(actor)さんほか\(entry.actorIDs.count - 1)人" : actor
    }
    private var currentPreview: FetchedPreview? {
        fetchedPreview?.noticeID == entry.noticeID ? fetchedPreview : nil
    }
    private var context: InboxContext? { entry.joinedContext ?? currentPreview?.context }
    private var canOpenChat: Bool {
        hasChannel && (missedCall || (entry.messageID != nil && context?.content != nil))
    }
    private var reactionGroups: [ReactionGroup] {
        guard entry.kind == "reaction", let context else { return [] }
        let actors = Set(entry.actorIDs)
        let groups = Dictionary(grouping: context.reactions.filter { actors.contains($0.user_id) }) {
            workspace.emoji(for: $0.emoji).map { ":\($0.name):" } ?? ReactionEmoji.key($0.emoji)
        }
        return groups.map { emoji, rows in
            ReactionGroup(emoji: emoji, count: Set(rows.map(\.user_id)).count, isMine: false)
        }.sorted { $0.emoji < $1.emoji }
    }
    private var channelTitle: String {
        if let channel { return channel.is_dm ? workspace.name(for: channel) : channel.channel_name }
        if let name = entry.channelName {
            return entry.kind == "dm" || name.hasPrefix("dm:") ? "ダイレクトメッセージ" : name
        }
        return "チャンネルを利用できません"
    }

    private var event: (text: String, symbol: String) {
        switch entry.kind {
        case "mention": ("メンション", "at")
        case "dm": ("ダイレクトメッセージ", "bubble.left.and.bubble.right")
        case "reply": (entry.channelID == nil ? "投稿への返信" : "スレッドへの返信", "arrowshape.turn.up.left")
        case "reaction", "like": ("リアクション", "face.smiling")
        case "missed_call": ("不在着信", "phone.down")
        case "follow_request": ("フォロー申請", "person.badge.plus")
        case "follow_accepted": ("フォローが承認されました", "person.crop.circle.badge.checkmark")
        case "repost": ("リポスト", "arrow.2.squarepath")
        default: ("通知", "bell")
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let channelID = entry.channelID {
                Button {
                    opening = true
                    Task {
                        openFailed = !(await openChat(channelID, missedCall ? nil : entry.messageID))
                        opening = false
                    }
                } label: {
                    VStack(alignment: .leading, spacing: 8) {
                        header()
                        if !missedCall {
                            if hasChannel, let text = context?.content {
                                preview(text)
                            } else if hasChannel && entry.joinedContext == nil && currentPreview == nil {
                                ProgressView().controlSize(.small)
                            } else if currentPreview?.failed != true {
                                Text("メッセージが削除されたか、閲覧できません").font(.subheadline).foregroundStyle(.secondary)
                            }
                        }
                        let groups = reactionGroups
                        if !groups.isEmpty {
                            LazyVGrid(columns: [GridItem(.adaptive(minimum: 76), alignment: .leading)], alignment: .leading) {
                                ForEach(groups) { group in
                                    let glyph = ReactionGlyph(emoji: group.emoji, message: nil, workspace: workspace)
                                    HStack(spacing: 4) {
                                        glyph
                                        Text(group.count, format: .number).font(.caption.monospacedDigit())
                                    }
                                    .accessibilityElement(children: .ignore)
                                    .accessibilityLabel("\(glyph.label)、\(group.count)人")
                                }
                            }
                        }
                        Label(channelTitle, systemImage: channel?.is_dm == true ? "bubble.left" : "number")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(opening || !canOpenChat)
                .accessibilityIdentifier("inbox-open-\(entry.messageID ?? channelID)")
                if currentPreview?.failed == true {
                    Button("プレビューを再読み込み") { retry += 1 }.font(.caption)
                }
                if openFailed { Text("会話を開けませんでした。もう一度お試しください。").font(.caption).foregroundStyle(.red) }
            } else {
                if let actorURL = store.actorURL(for: entry) {
                    Link(destination: actorURL) { header(actorColor: AppDesign.externalLink) }.buttonStyle(.plain)
                        .accessibilityIdentifier("inbox-actor-\(entry.id)")
                } else { header() }
                if let postURL = store.webURL(for: entry), postURL != store.actorURL(for: entry) {
                    Link(destination: postURL) {
                        VStack(alignment: .leading, spacing: 6) {
                            if let text = entry.content { preview(text) }
                            Label("投稿を開く", systemImage: "arrow.up.right").font(.caption).foregroundStyle(.tint)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .tint(AppDesign.externalLink)
                    .accessibilityIdentifier("inbox-post-\(entry.id)")
                } else if let text = entry.content { preview(text) }
                if entry.kind == "follow_request", entry.pendingFollow {
                    HStack {
                        Button("承認") { respond(true) }.buttonStyle(.borderedProminent)
                            .tint(AppDesign.primary)
                            .foregroundStyle(responding ? Color.secondary : AppDesign.primaryForeground)
                        Button("拒否") { respond(false) }.buttonStyle(.bordered)
                    }.disabled(responding)
                }
            }
        }
        .padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, AppDesign.spacing)
        .frame(maxWidth: .infinity, alignment: .leading)
        .labelStyle(.titleAndIcon)
        .onScrollVisibilityChange(threshold: 0.1) { visible = $0 }
        .onChange(of: isActive && visible, initial: true) { _, onScreen in
            store.setVisible(entry, visible: onScreen)
        }
        .onDisappear { store.setVisible(entry, visible: false) }
        .onChange(of: isActive && visible && !store.markingAll ? entry.unreadIDs : [], initial: true) { _, unread in
            if !unread.isEmpty { Task { await store.markRead(entry) } }
        }
        .onChange(of: store.refreshRevision) { _, _ in
            if isActive, visible, !store.markingAll, entry.isUnread { Task { await store.markRead(entry) } }
        }
        .task(id: ContextRequest(noticeID: needsPreview ? entry.noticeID : nil, retry: retry)) {
            guard needsPreview else { return }
            do {
                let result = try await store.context(for: entry)
                guard !Task.isCancelled else { return }
                fetchedPreview = FetchedPreview(noticeID: entry.noticeID, context: result, failed: false)
            } catch {
                guard !Task.isCancelled else { return }
                fetchedPreview = FetchedPreview(noticeID: entry.noticeID, context: nil, failed: true)
            }
        }
    }

    private var needsPreview: Bool {
        isActive && visible && hasChannel && !missedCall && entry.channelID != nil && entry.joinedContext == nil
    }
    private struct FetchedPreview {
        let noticeID: String
        let context: InboxContext?
        let failed: Bool
    }
    private struct ContextRequest: Equatable {
        let noticeID: String?
        let retry: Int
    }

    private func header(actorColor: Color = .primary) -> some View {
        HStack(alignment: .top, spacing: AppDesign.spacing) {
            AvatarView(name: actor, url: entry.avatarURL ?? person?.avatarURL)
            VStack(alignment: .leading, spacing: 3) {
                Text(actorSummary).font(.subheadline.bold()).foregroundStyle(actorColor)
                Label {
                    Text(event.text)
                } icon: {
                    if entry.kind == "like" {
                        ReactionGlyph(emoji: entry.reaction ?? "❤️", message: nil, workspace: workspace)
                    } else {
                        Image(systemName: event.symbol)
                    }
                }
                .font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 2)
            VStack(alignment: .trailing, spacing: 3) {
                Text(entry.createdAt, format: .relative(presentation: .numeric))
                    .font(.caption2).foregroundStyle(.secondary)
                if entry.isUnread {
                    Circle().fill(.tint).frame(width: 8, height: 8).accessibilityLabel("未読")
                }
            }
        }
    }

    private func preview(_ content: String) -> some View {
        Text(content.isEmpty ? "添付ファイルまたはアプリのメッセージ" : String(content.prefix(280)))
            .font(.subheadline).foregroundStyle(.primary).lineLimit(3)
    }

    private func respond(_ accept: Bool) {
        responding = true
        Task {
            await store.respond(entry, accept: accept)
            responding = false
        }
    }
}
