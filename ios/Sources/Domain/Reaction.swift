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
    var attachments: [MessageAttachment] = []
    var reply_count: Int? = nil
}

extension MessageDisplayData {
    private struct Fields: Decodable {
        var reactions: [MessageReaction]?
        var emojis: [String: DisplayEmoji]?
        var attachments: [MessageAttachment]?
        var reply_count: Int?

        private enum CodingKeys: String, CodingKey { case reactions, emojis, attachments, reply_count }

        init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            reactions = try? container.decodeIfPresent([MessageReaction].self, forKey: .reactions)
            emojis = try? container.decodeIfPresent([String: DisplayEmoji].self, forKey: .emojis)
            attachments = try? container.decodeIfPresent([MessageAttachment].self, forKey: .attachments)
            reply_count = try? container.decodeIfPresent(Int.self, forKey: .reply_count)
        }
    }

    init(from decoder: any Decoder) throws {
        // Electric の JSONB はサーバーの版によって文字列またはオブジェクトで届く。
        let fields: Fields?
        if let raw = try? decoder.singleValueContainer().decode(String.self) {
            fields = try? JSONDecoder().decode(Fields.self, from: Data(raw.utf8))
        } else {
            fields = try? Fields(from: decoder)
        }
        self.init(reactions: fields?.reactions, emojis: fields?.emojis ?? [:], attachments: fields?.attachments ?? [], reply_count: fields?.reply_count)
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
