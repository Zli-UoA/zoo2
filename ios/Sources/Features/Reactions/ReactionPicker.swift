import SwiftUI

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
            AppList {
                if query.isEmpty {
                    Picker("スタンプの種類", selection: $category) {
                        ForEach(Category.allCases, id: \.self) { category in
                            Text(category.rawValue).tag(category)
                        }
                    }.pickerStyle(.segmented)
                        .listRowInsets(AppDesign.rowInsets).listRowBackground(Color.clear)
                }
                if query.isEmpty, category == .standard {
                    let recent = workspace.recentEmojis.values.filter { value in
                        if value.hasPrefix(":@") { return workspace.emojiAccounts.contains { ":@" + $0.username + ":" == value } }
                        return !value.hasPrefix(":") || workspace.emoji(for: value) != nil
                    }
                    if !recent.isEmpty {
                        Section("最近使った絵文字") {
                            ForEach(recent.prefix(6), id: \.self) { value in
                                let glyph = ReactionGlyph(emoji: value, message: nil, workspace: workspace)
                                Button { select(value) } label: { glyph }
                                    .accessibilityLabel(glyph.label)
                            }
                        }
                    }
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
                                HStack(spacing: AppDesign.spacing) {
                                    CachedRemoteImage(url: emoji.imageURL, policy: .stored) { image in
                                        if let image { image.resizable().scaledToFit() }
                                        else { Image(systemName: "face.smiling").foregroundStyle(.secondary) }
                                    }
                                    .frame(width: 36, height: 32).accessibilityHidden(true)
                                    Text(":\(emoji.name):").foregroundStyle(.primary)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                            }
                            .accessibilityIdentifier("pick-reaction-:\(emoji.name):")
                        }
                    }
                }
                if !accounts.isEmpty {
                    Section("アカウントのスタンプ") {
                        ForEach(accounts) { account in
                            Button { select(":@\(account.username):") } label: {
                                HStack(spacing: AppDesign.spacing) {
                                    AccountStamp(account: account, username: account.username, size: 32)
                                    VStack(alignment: .leading, spacing: 2) {
                                        if !account.display_name.isEmpty { Text(account.display_name).foregroundStyle(.primary) }
                                        Text("@\(account.username)").font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                                .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
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
            .appSearch(text: $search, prompt: "絵文字を入力、または名前で検索", identifier: "reaction")
            .toolbar { ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } } }
        }
        .onAppear { workspace.recentEmojis.refresh() }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }
}
