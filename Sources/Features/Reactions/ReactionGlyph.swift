import SwiftUI

struct ReactionGlyph: View {
    let emoji: String
    let message: Message?
    let workspace: WorkspaceStore
    @State private var kitchenImage: URL?
    @ScaledMetric(relativeTo: .body) private var size = 24.0

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
        return workspace.emojiImageURL(for: emoji, message: message)
    }

    var body: some View {
        Group {
            if let username {
                AccountStamp(account: workspace.emojiAccount(named: username), username: username, size: size)
            } else if let imageURL {
                AsyncImage(url: imageURL) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFit().frame(width: size, height: size)
                    } else { fallback }
                }
            } else { fallback }
        }
        .frame(width: size, height: size)
        .task(id: emoji) {
            guard emoji.hasPrefix("kitchen:") else { return }
            let url = await ReactionEmoji.kitchenImageURL(emoji)
            if !Task.isCancelled { kitchenImage = url }
        }
    }

    private var fallback: some View {
        Text(label).font(label.count == 1 ? .system(size: size) : .caption)
            .lineLimit(1).minimumScaleFactor(0.5)
    }
}

struct AccountStamp: View {
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
