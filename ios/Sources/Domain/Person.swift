import Foundation

struct Person: Codable, Identifiable, Hashable, Sendable {
    let id: String
    var user_name: String
    var display_name: String
    var is_bot: Bool = false
    var is_admin: Bool? = nil
    var is_deactivated: Bool = false
    var avatar_url: String? = nil

    var name: String { display_name.isEmpty ? user_name : display_name }

    var avatarURL: URL? {
        guard let avatar_url, avatar_url.utf8.count <= 2048,
              let url = URL(string: avatar_url), url.scheme?.lowercased() == "https",
              let host = url.host, !host.isEmpty, url.user == nil, url.password == nil,
              url.absoluteString.utf8.count <= 2048 else { return nil }
        return url
    }
}

extension Person {
    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        user_name = try values.decode(String.self, forKey: .user_name)
        display_name = try values.decode(String.self, forKey: .display_name)
        avatar_url = try values.decodeIfPresent(String.self, forKey: .avatar_url)
        // REST は Bool、Electric は PostgreSQL の真偽値を文字列で返す。
        func flag(_ key: CodingKeys) throws -> Bool? {
            guard let raw = try values.decodeIfPresent(JSONValue.self, forKey: key) else { return nil }
            switch raw {
            case .bool(let value): return value
            case .string("true"), .string("t"): return true
            case .string("false"), .string("f"): return false
            default:
                throw DecodingError.dataCorruptedError(forKey: key, in: values, debugDescription: "ユーザー属性の真偽値を読み取れません。")
            }
        }
        guard let bot = try flag(.is_bot) else { throw DecodingError.keyNotFound(CodingKeys.is_bot, .init(codingPath: decoder.codingPath, debugDescription: "ユーザー種別がありません。")) }
        is_bot = bot
        is_admin = try flag(.is_admin)
        is_deactivated = try flag(.is_deactivated) ?? false
    }
}
