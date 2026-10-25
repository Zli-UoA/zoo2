import SwiftUI

struct ChannelScreen: View {
    @Environment(ZooStore.self) private var store
    let route: ChannelRoute
    @State private var thread: Message?
    @State private var showMembers = false
    @State private var consumedTarget = false
    @State private var busy = false
    private var channel: Channel { store.channel(route.channel.id) ?? route.channel }
    private var canPost: Bool { store.channel(channel.id) != nil && store.joined(channel.id) && channel.archive_at == nil && store.syncErrors["channel-access"] == nil }

    var body: some View {
        VStack(spacing: 0) {
            if let description = channel.description, !description.isEmpty {
                Text(description).font(.caption).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18).padding(.vertical, 8)
                Divider()
            }
            if store.syncErrors["channel-access"] != nil {
                ContentUnavailableView("チャンネルを表示できません", systemImage: "lock", description: Text("参加状態を確認するか、再接続してください。"))
            } else {
                MessageTimeline(channelID: channel.id, parentID: nil, targetID: route.messageID, canPost: canPost) { thread = $0 }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if canPost { Composer(channelID: channel.id, parentID: nil) }
            else if channel.archive_at != nil {
                Label("アーカイブ済みのチャンネルです", systemImage: "archivebox").font(.callout).padding()
            } else if !channel.is_private && !channel.is_dm && store.channel(channel.id) != nil {
                Button("チャンネルに参加") {
                    busy = true
                    Task { _ = await store.perform { try await store.join(channel) }; busy = false }
                }.buttonStyle(.borderedProminent).padding().disabled(busy)
            }
        }
        .navigationTitle(store.title(channel)).navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) { Button("メンバー", systemImage: "person.2") { showMembers = true } }
        }
        .task(id: channel.id) { store.selectChannel(channel.id) }
        .onDisappear { store.closeChannel(channel.id) }
        .onChange(of: store.timeline, initial: true) { _, rows in
            guard !consumedTarget, let target = route.messageID,
                  let message = rows.first(where: { $0.id == target }) else { return }
            if let parent = message.parent_id { thread = rows.first { $0.id == parent } }
            consumedTarget = true
        }
        .sheet(item: $thread) { root in
            NavigationStack {
                VStack(spacing: 0) {
                    MessageTimeline(channelID: channel.id, parentID: root.id, targetID: route.messageID, canPost: canPost) { _ in }
                }
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    if canPost && store.timeline.first(where: { $0.id == root.id })?.deleted_at == nil {
                        Composer(channelID: channel.id, parentID: root.id)
                    }
                }
                .navigationTitle("スレッド").navigationBarTitleDisplayMode(.inline)
                .toolbar { ToolbarItem(placement: .confirmationAction) { Button("閉じる") { thread = nil } } }
            }.presentationDragIndicator(.visible)
        }
        .sheet(isPresented: $showMembers) {
            ChannelSettingsScreen(channelID: channel.id)
        }
    }
}

struct MessageTimeline: View {
    @Environment(ZooStore.self) private var store
    let channelID: String
    let parentID: String?
    let targetID: String?
    let canPost: Bool
    let openThread: (Message) -> Void
    @State private var limit = 100
    @State private var atBottom = true
    @State private var positioned = false
    @State private var markingRead = false

    private var rows: [Message] {
        store.timeline.filter { $0.channel_id == channelID && ($0.parent_id == parentID || $0.id == parentID) }
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if rows.count > limit {
                        Button("以前のメッセージを表示") { limit += 100 }.frame(maxWidth: .infinity).padding()
                    }
                    if store.loadingMessages && rows.isEmpty {
                        ProgressView("メッセージを読み込み中").frame(maxWidth: .infinity).padding(40)
                    } else if rows.isEmpty {
                        ContentUnavailableView(parentID == nil ? "ここから会話をはじめよう" : "最初の返信を送ろう", systemImage: "bubble.left", description: Text("アイデアも、ちょっとした一言も。"))
                    }
                    ForEach(Array(rows.suffix(limit))) { message in
                        MessageRow(message: message, canPost: canPost, showsThread: parentID == nil, openThread: openThread)
                            .id(message.id)
                    }
                    Color.clear.frame(height: 1).id("bottom")
                        .onAppear { atBottom = true }.onDisappear { atBottom = false }
                    if parentID == nil, let last = rows.last(where: { $0.deleted_at == nil }) {
                        Button("ここまで既読にする") {
                            markingRead = true
                            Task { _ = await store.perform { try await store.markRead(channelID, messageID: last.id) }; markingRead = false }
                        }.font(.caption).padding().frame(maxWidth: .infinity).disabled(markingRead)
                    }
                }.padding(.vertical, 10)
            }
            .scrollDismissesKeyboard(.interactively)
            .defaultScrollAnchor(.bottom)
            .onChange(of: rows, initial: true) { _, new in
                guard !new.isEmpty else { return }
                if !positioned {
                    if let targetID, let index = new.firstIndex(where: { $0.id == targetID }) {
                        limit = max(limit, new.count - index)
                        Task { @MainActor in
                            await Task.yield()
                            proxy.scrollTo(targetID, anchor: .center)
                        }
                    } else { proxy.scrollTo("bottom", anchor: .bottom) }
                    positioned = true
                } else if atBottom { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
            }
            .overlay(alignment: .bottomTrailing) {
                if !atBottom && !rows.isEmpty {
                    Button("最新へ", systemImage: "arrow.down") {
                        withAnimation { proxy.scrollTo("bottom", anchor: .bottom) }
                    }.font(.caption.bold()).buttonStyle(.borderedProminent).clipShape(Capsule()).padding()
                }
            }
        }
    }
}

struct MessageRow: View {
    @Environment(ZooStore.self) private var store
    let message: Message
    let canPost: Bool
    let showsThread: Bool
    let openThread: (Message) -> Void
    @State private var editing = false
    @State private var deleting = false
    @State private var busy = false
    private var author: String { store.name(message.user_id) }
    private var emoji: [String] { Array(Set(store.reactions.filter { $0.message_id == message.id }.map(\.emoji))).sorted() }
    private var replies: Int { store.replyCounts[message.id, default: 0] }

    var body: some View {
        HStack(alignment: .top, spacing: 11) {
            Avatar(name: author)
            VStack(alignment: .leading, spacing: 7) {
                HStack(alignment: .firstTextBaseline, spacing: 7) {
                    Text(author).font(.subheadline.bold())
                    if store.person(message.user_id)?.is_bot == true { Text("BOT").font(.caption2.monospaced()).foregroundStyle(.secondary) }
                    if let date = ZooDate.parse(message.created_at) { Text(date, style: .time).font(.caption2).foregroundStyle(.secondary) }
                    if message.updated_at != nil { Text("編集済み").font(.caption2).foregroundStyle(.secondary) }
                }
                if message.deleted_at != nil { Text("メッセージは削除されました").font(.callout).foregroundStyle(.secondary).italic() }
                else {
                    Text(message.content).font(.body).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityIdentifier("message-\(message.id)")
                    if let raw = message.card, let card = IntegrationCard.parse(raw) { IntegrationCardView(card: card) }
                    if !emoji.isEmpty {
                        ViewThatFits(in: .horizontal) {
                            reactionButtons
                            ScrollView(.horizontal, showsIndicators: false) { reactionButtons }
                        }
                    }
                    if showsThread && replies > 0 {
                        Button { openThread(message) } label: {
                            Label("\(replies) 件の返信", systemImage: "bubble.left.and.bubble.right").font(.caption.bold())
                        }.padding(.vertical, 5).accessibilityIdentifier("thread-\(message.id)")
                    }
                }
            }
        }
        .padding(.horizontal, 18).padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .contextMenu {
            if message.deleted_at == nil {
                Button("コピー", systemImage: "doc.on.doc") { UIPasteboard.general.string = message.content }
                if showsThread { Button("スレッドを開く", systemImage: "bubble.left") { openThread(message) } }
                if canPost {
                    Menu("リアクション", systemImage: "face.smiling") {
                        ForEach(["👍", "❤️", "🎉", "👀", "🙌", "☕️"], id: \.self) { emoji in
                            Button(emoji) { react(emoji) }.disabled(busy)
                        }
                    }
                    if message.user_id == store.me?.id {
                        Button("編集", systemImage: "pencil") { editing = true }
                        Button("削除", systemImage: "trash", role: .destructive) { deleting = true }
                    }
                }
            }
        }
        .confirmationDialog("このメッセージを削除しますか？", isPresented: $deleting, titleVisibility: .visible) {
            Button("削除", role: .destructive) { Task { _ = await store.perform { try await store.delete(message) } } }
        }
        .sheet(isPresented: $editing) { EditMessageScreen(message: message, text: message.content) }
    }

    private var reactionButtons: some View {
        HStack(spacing: 6) {
            ForEach(emoji, id: \.self) { symbol in
                let matches = store.reactions.filter { $0.message_id == message.id && $0.emoji == symbol }
                let mine = matches.contains { $0.user_id == store.me?.id }
                Button { react(symbol) } label: {
                    HStack(spacing: 4) { Text(symbol); Text("\(matches.count)").font(.caption.weight(.medium)) }
                        .padding(.horizontal, 10).frame(minHeight: 44)
                        .background(mine ? Color.zooAccent.opacity(0.12) : Color(uiColor: .secondarySystemBackground), in: Capsule())
                }.buttonStyle(.plain).disabled(!canPost || busy)
                    .accessibilityLabel("\(symbol) \(matches.count) 件\(mine ? "、リアクション済み" : "")")
            }
        }
    }

    private func react(_ emoji: String) {
        busy = true
        Task { _ = await store.perform { try await store.react(message, emoji: emoji) }; busy = false }
    }
}

struct IntegrationCard: Decodable {
    let title: String
    let description: String?
    let url: String?
    let source: String?
    let context: String?
    let author: String?
    let badge: String?

    static func parse(_ raw: String) -> Self? {
        guard let card = try? JSONDecoder().decode(Self.self, from: Data(raw.utf8)),
              !card.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              card.title.count <= 2000 else { return nil }
        return card
    }

    var destination: URL? {
        guard let url, let target = URL(string: url), target.scheme == "https", target.host != nil,
              target.user == nil, target.password == nil else { return nil }
        return target
    }
}

struct IntegrationCardView: View {
    let card: IntegrationCard
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text([card.source, card.context, card.badge].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                .font(.caption).foregroundStyle(.secondary)
            if let target = card.destination { Link(card.title, destination: target).font(.subheadline.bold()) }
            else { Text(card.title).font(.subheadline.bold()) }
            if let description = card.description, !description.isEmpty { Text(description).font(.callout).lineLimit(8) }
            if let author = card.author, !author.isEmpty { Text(author).font(.caption).foregroundStyle(.secondary) }
        }.padding(12).frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(uiColor: .secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
            .overlay(alignment: .leading) { UnevenRoundedRectangle(topLeadingRadius: 12, bottomLeadingRadius: 12).fill(Color.zooAccent).frame(width: 3) }
    }
}

struct Composer: View {
    @Environment(ZooStore.self) private var store
    let channelID: String
    let parentID: String?
    @State private var sending = false
    @FocusState private var focused: Bool
    private var key: String { "\(channelID)/\(parentID ?? "root")" }
    private var text: Binding<String> { Binding(get: { store.drafts[key] ?? "" }, set: { store.drafts[key] = $0 }) }

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(alignment: .bottom, spacing: 10) {
                TextField(parentID == nil ? "メッセージを送る" : "スレッドに返信", text: text, axis: .vertical)
                    .lineLimit(1...6).padding(12)
                    .background(Color(uiColor: .secondarySystemBackground), in: RoundedRectangle(cornerRadius: 20))
                    .focused($focused).accessibilityIdentifier(parentID == nil ? "composer" : "replyComposer")
                Button {
                    let submitted = text.wrappedValue
                    sending = true
                    Task {
                        let success = await store.perform { try await store.send(submitted, channelID: channelID, parentID: parentID) }
                        if success && text.wrappedValue == submitted { text.wrappedValue = "" }
                        sending = false
                    }
                } label: {
                    Group {
                        if sending { ProgressView().tint(.white) }
                        else { Image(systemName: "arrow.up").font(.title3.bold()) }
                    }.foregroundStyle(.white).frame(width: 44, height: 44)
                        .background(Color.zooAccent, in: Circle())
                }.disabled(sending || text.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || text.wrappedValue.unicodeScalars.count > 40_000)
                    .opacity(text.wrappedValue.isEmpty ? 0.4 : 1)
                    .accessibilityLabel("送信").accessibilityIdentifier(parentID == nil ? "sendButton" : "sendReplyButton")
            }.padding(.horizontal, 14).padding(.vertical, 10)
        }.background(.bar)
    }
}

struct EditMessageScreen: View {
    @Environment(ZooStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let message: Message
    @State var text: String
    @State private var busy = false

    var body: some View {
        NavigationStack {
            TextEditor(text: $text).padding().accessibilityIdentifier("editMessage")
                .navigationTitle("メッセージを編集").navigationBarTitleDisplayMode(.inline)
                .interactiveDismissDisabled(busy)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("キャンセル") { dismiss() }.disabled(busy) }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("保存") {
                            busy = true
                            Task {
                                if await store.perform({ try await store.edit(message, content: text) }) { dismiss() }
                                busy = false
                            }
                        }.disabled(busy || text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || text.unicodeScalars.count > 40_000)
                    }
                }
        }
    }
}
