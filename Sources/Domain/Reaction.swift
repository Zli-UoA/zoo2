import Foundation

struct MessageReaction: Codable, Identifiable, Hashable, Sendable {
    let id: String
    let message_id: String
    let user_id: String
    let emoji: String
}

struct DisplayEmoji: Codable, Hashable, Sendable {
    var url: String?
    var image_id: String?

    func imageURL(relativeTo server: String) -> URL? {
        Self.assetURL(url, relativeTo: server)
    }

    static func assetURL(_ raw: String?, relativeTo server: String? = nil) -> URL? {
        guard var path = raw, !path.isEmpty else { return nil }
        if path.hasPrefix("/"), !path.hasPrefix("//"), let server {
            path = server.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + path
        }
        guard let url = URL(string: path), url.scheme?.lowercased() == "https",
              let host = url.host, !host.isEmpty, url.user == nil, url.password == nil else { return nil }
        return url
    }
}

struct MessageDisplayData: Codable, Hashable, Sendable {
    var reactions: [MessageReaction]?
    var emojis: [String: DisplayEmoji] = [:]
}

extension MessageDisplayData {
    private struct Fields: Decodable {
        var reactions: [MessageReaction]?
        var emojis: [String: DisplayEmoji]?
    }

    init(from decoder: any Decoder) throws {
        // Electric の JSONB はサーバーの版によって文字列またはオブジェクトで届く。
        let fields: Fields?
        if let raw = try? decoder.singleValueContainer().decode(String.self) {
            fields = try? JSONDecoder().decode(Fields.self, from: Data(raw.utf8))
        } else {
            fields = try? Fields(from: decoder)
        }
        self.init(reactions: fields?.reactions, emojis: fields?.emojis ?? [:])
    }
}

struct WorkspaceEmoji: Codable, Identifiable, Hashable, Sendable {
    let name: String
    var url: String
    let category: String
    let aliases: [String]

    var id: String { name }
    var imageURL: URL? { DisplayEmoji.assetURL(url) }

    func resolved(relativeTo server: String) -> Self {
        var result = self
        if let absolute = DisplayEmoji.assetURL(url, relativeTo: server) {
            result.url = absolute.absoluteString
        }
        return result
    }
}

struct EmojiAccount: Codable, Identifiable, Hashable, Sendable {
    let username: String
    let display_name: String
    var avatar_url: String?

    var id: String { username }
    var avatarURL: URL? { DisplayEmoji.assetURL(avatar_url) }

    func resolved(relativeTo server: String) -> Self {
        var result = self
        result.avatar_url = DisplayEmoji.assetURL(avatar_url, relativeTo: server)?.absoluteString
        return result
    }
}
