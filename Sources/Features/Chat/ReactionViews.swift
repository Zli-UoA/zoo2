import SwiftUI

struct ReactionChips: View {
    let store: ConversationStore
    let workspace: WorkspaceStore
    let message: Message

    var body: some View {
        let groups = store.reactions(for: message.id)
        if !groups.isEmpty {
            ScrollView(.horizontal) {
                HStack(spacing: 5) {
                    ForEach(groups, id: \.emoji) { group in
                        let glyph = ReactionGlyph(emoji: group.emoji, message: message, workspace: workspace)
                        let selected = group.isMine
                        Button {
                            Task { await store.toggleReaction(messageID: message.id, emoji: group.emoji) }
                        } label: {
                            HStack(spacing: 4) {
                                glyph
                                Text(group.count, format: .number).font(.caption.bold().monospacedDigit())
                            }
                            .padding(.horizontal, 8).padding(.vertical, 4)
                            .foregroundStyle(selected ? Color.accentColor : Color.primary)
                            .background(selected ? Color.accentColor.opacity(0.12) : Color(.secondarySystemBackground), in: Capsule())
                            .overlay { Capsule().strokeBorder(selected ? Color.accentColor : Color(.separator), lineWidth: 1) }
                            .frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(!store.canReact(to: message.id) || group.emoji.hasPrefix("apub:") ||
                                  store.isReactionPending(messageID: message.id, emoji: group.emoji))
                        .accessibilityLabel(glyph.label)
                        .accessibilityValue(selected ? "\(group.count)人、自分も付けています" : "\(group.count)人")
                        .accessibilityAddTraits(selected ? .isSelected : [])
                        .accessibilityIdentifier("reaction-\(message.id)-\(group.emoji)")
                    }
                }
            }
            .scrollIndicators(.hidden)
        }
    }
}

struct ReactionGlyph: View {
    let emoji: String
    let message: Message?
    let workspace: WorkspaceStore
    @State private var kitchenImage: URL?

    private var username: String? {
        guard emoji.hasPrefix(":@"), emoji.hasSuffix(":"), emoji.count > 3 else { return nil }
        return String(emoji.dropFirst(2).dropLast())
    }

    private var remoteImageID: String? {
        guard emoji.hasPrefix("apub:") else { return nil }
        let parts = emoji.split(separator: ":", maxSplits: 2, omittingEmptySubsequences: false)
        guard parts.count == 3, parts[1].count == 64,
              parts[1].utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { return nil }
        return String(parts[1])
    }

    var label: String {
        if let username {
            let name = workspace.emojiAccount(named: username)?.display_name ?? ""
            return name.isEmpty ? "@\(username) のスタンプ" : "\(name)（@\(username)）のスタンプ"
        }
        if remoteImageID != nil { return String(emoji.dropFirst(70)) }
        return ReactionEmoji.kitchenLabel(emoji) ?? emoji
    }

    private var imageURL: URL? {
        if emoji.hasPrefix("kitchen:") { return kitchenImage }
        if let id = remoteImageID {
            return (try? ConnectionSettings.secureURL(workspace.client.serverURL))?.appending(path: "api/activitypub/emoji-images/\(id)")
        }
        guard emoji.hasPrefix(":"), emoji.hasSuffix(":"), emoji.count > 2 else { return nil }
        let name = String(emoji.dropFirst().dropLast())
        let custom = workspace.emoji(for: emoji)
        if let attached = message?.display_data?.emojis {
            for spelling in [name] + (custom?.aliases ?? []) {
                if let url = attached[spelling]?.imageURL(relativeTo: workspace.client.serverURL) { return url }
            }
        }
        return custom?.imageURL
    }

    var body: some View {
        Group {
            if let username {
                AccountStamp(account: workspace.emojiAccount(named: username), username: username, size: 24)
            } else if let imageURL {
                AsyncImage(url: imageURL) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFit().frame(maxWidth: 86, maxHeight: 24)
                    } else { fallback }
                }
            } else { fallback }
        }
        .task(id: emoji) {
            guard emoji.hasPrefix("kitchen:") else { return }
            let url = await ReactionEmoji.kitchenImageURL(emoji)
            if !Task.isCancelled { kitchenImage = url }
        }
    }

    private var fallback: some View {
        Text(label).font(label.count == 1 ? .body : .caption)
            .lineLimit(1).frame(maxWidth: 140)
    }
}

private struct AccountStamp: View {
    let account: EmojiAccount?
    let username: String
    let size: CGFloat

    var body: some View {
        AsyncImage(url: account?.avatarURL) { phase in
            if let image = phase.image {
                image.resizable().scaledToFill()
            } else {
                let name = account?.display_name ?? ""
                Text(String((name.isEmpty ? username : name).prefix(1)))
                    .font(.system(size: size * 0.55, weight: .semibold)).foregroundStyle(.tint)
            }
        }
        .frame(width: size, height: size).background(.tint.opacity(0.12)).clipShape(.circle)
        .accessibilityHidden(true)
    }
}

struct ReactionPicker: View {
    let workspace: WorkspaceStore
    let select: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""
    @State private var category = Category.standard

    private enum Category: String, CaseIterable {
        case standard = "絵文字"
        case custom = "カスタム"
        case accounts = "アカウント"
    }

    private static let standard = [
        ("👍", "いいね"), ("❤️", "ハート"), ("🎉", "お祝い"), ("👏", "拍手"), ("🙌", "やった"),
        ("😂", "笑い"), ("🙏", "ありがとう"), ("👀", "見ています"), ("😊", "笑顔"), ("🔥", "炎"),
        ("💯", "満点"), ("✨", "きらきら"), ("🤔", "考え中"), ("🚀", "ロケット"), ("✅", "確認"),
    ]

    private var query: String { search.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var standard: [(String, String)] {
        guard !query.isEmpty || category == .standard else { return [] }
        var options = Self.standard.filter { query.isEmpty || $0.0 == query || $0.1.localizedCaseInsensitiveContains(query) }
        if options.isEmpty, query.count == 1, query.utf8.count <= 256,
           query.unicodeScalars.contains(where: { ($0.properties.isEmoji && !$0.isASCII) || $0.value == 0x20E3 }) {
            options.append((query, query))
        }
        return options
    }

    private var custom: [WorkspaceEmoji] {
        guard !query.isEmpty || category == .custom else { return [] }
        let name = query.trimmingCharacters(in: CharacterSet(charactersIn: ":"))
        return workspace.emojis.filter { emoji in
            name.isEmpty || ([emoji.name, emoji.category] + emoji.aliases).contains { $0.localizedCaseInsensitiveContains(name) }
        }
    }

    private var accounts: [EmojiAccount] {
        guard !query.isEmpty || category == .accounts else { return [] }
        let name = query.trimmingCharacters(in: CharacterSet(charactersIn: ":@"))
        return workspace.emojiAccounts.filter {
            name.isEmpty || $0.username.localizedCaseInsensitiveContains(name) || $0.display_name.localizedCaseInsensitiveContains(name)
        }
    }

    var body: some View {
        let standard = standard
        let custom = custom
        let accounts = accounts
        NavigationStack {
            List {
                if query.isEmpty {
                    Picker("スタンプの種類", selection: $category) {
                        ForEach(Category.allCases, id: \.self) { category in
                            Text(category.rawValue).tag(category)
                        }
                    }.pickerStyle(.segmented)
                        .listRowInsets(EdgeInsets()).listRowBackground(Color.clear)
                }
                if !standard.isEmpty {
                    Section("絵文字") {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 44), spacing: 4)], spacing: 4) {
                            ForEach(standard, id: \.0) { emoji, name in
                                Button { select(emoji) } label: {
                                    Text(emoji).font(.title2)
                                        .frame(maxWidth: .infinity, minHeight: 44).contentShape(Rectangle())
                                }
                                .buttonStyle(.plain).accessibilityLabel(name)
                                .accessibilityIdentifier("pick-reaction-\(emoji)")
                            }
                        }
                    }
                }
                if !custom.isEmpty {
                    Section("ワークスペースのスタンプ") {
                        ForEach(custom) { emoji in
                            Button { select(":\(emoji.name):") } label: {
                                HStack(spacing: 12) {
                                    AsyncImage(url: emoji.imageURL) { phase in
                                        if let image = phase.image { image.resizable().scaledToFit() }
                                        else { Image(systemName: "face.smiling").foregroundStyle(.secondary) }
                                    }
                                    .frame(width: 36, height: 32).accessibilityHidden(true)
                                    Text(":\(emoji.name):").foregroundStyle(.primary)
                                }
                            }
                            .accessibilityIdentifier("pick-reaction-:\(emoji.name):")
                        }
                    }
                }
                if !accounts.isEmpty {
                    Section("アカウントのスタンプ") {
                        ForEach(accounts) { account in
                            Button { select(":@\(account.username):") } label: {
                                HStack(spacing: 12) {
                                    AccountStamp(account: account, username: account.username, size: 32)
                                    VStack(alignment: .leading) {
                                        if !account.display_name.isEmpty { Text(account.display_name).foregroundStyle(.primary) }
                                        Text("@\(account.username)").font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                            }
                            .accessibilityIdentifier("pick-reaction-:@\(account.username):")
                        }
                    }
                }
                if let error = workspace.emojiError, category == .custom || !query.isEmpty {
                    Section {
                        Text(error).font(.callout)
                        Button("スタンプ一覧を再読み込み") { Task { await workspace.loadEmojis() } }
                    }
                }
                if let error = workspace.emojiAccountError, category == .accounts || !query.isEmpty {
                    Section {
                        Text(error).font(.callout)
                        Button("アカウント一覧を再読み込み") { Task { await workspace.loadEmojiAccounts() } }
                    }
                }
                if standard.isEmpty && custom.isEmpty && accounts.isEmpty {
                    if !query.isEmpty { ContentUnavailableView.search(text: query) }
                    else if category == .custom, workspace.emojiError == nil {
                        Text("ワークスペースのスタンプはありません。").foregroundStyle(.secondary)
                    } else if category == .accounts, workspace.emojiAccountError == nil {
                        Text("アカウントのスタンプはありません。").foregroundStyle(.secondary)
                    }
                }
            }
            .navigationTitle("スタンプを追加").navigationBarTitleDisplayMode(.inline)
            .searchable(text: $search, placement: .navigationBarDrawer(displayMode: .always),
                        prompt: "絵文字を入力、または名前で検索")
            .textInputAutocapitalization(.never).autocorrectionDisabled()
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }
}
