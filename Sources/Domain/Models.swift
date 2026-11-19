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

struct Channel: Codable, Identifiable, Hashable, Sendable {
    let id: String
    var channel_name: String
    var description: String?
    var is_private: Bool
    var is_dm: Bool
    var archive_at: String?
    var deleted_at: String?
    var encrypted: Bool?
    var created_user_id: String? = nil
    var updated_at: String? = nil

    var canPost: Bool { archive_at == nil && deleted_at == nil && encrypted != true }
    var canCall: Bool { archive_at == nil && deleted_at == nil }
    var symbol: String { encrypted == true ? "lock.shield" : is_dm ? "bubble.left.and.bubble.right" : is_private ? "lock" : "number" }

    func canManage(by user: Person) -> Bool {
        !is_dm && deleted_at == nil && archive_at == nil && !user.is_deactivated &&
        (created_user_id == user.id || user.is_admin == true)
    }
}

struct ChannelReadState: Codable, Sendable {
    let channel_id: String
    var has_unread: Bool
    let last_read_at: String?
}

extension Channel {
    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        channel_name = try values.decode(String.self, forKey: .channel_name)
        description = try values.decodeIfPresent(String.self, forKey: .description)
        archive_at = try values.decodeIfPresent(String.self, forKey: .archive_at)
        deleted_at = try values.decodeIfPresent(String.self, forKey: .deleted_at)
        created_user_id = try values.decodeIfPresent(String.self, forKey: .created_user_id)
        updated_at = try values.decodeIfPresent(String.self, forKey: .updated_at)
        func flag(_ key: CodingKeys) throws -> Bool? {
            guard let raw = try values.decodeIfPresent(JSONValue.self, forKey: key) else { return nil }
            switch raw {
            case .bool(let value): return value
            case .string("true"), .string("t"): return true
            case .string("false"), .string("f"): return false
            default: throw ZooError.invalidResponse
            }
        }
        guard let privateFlag = try flag(.is_private), let dmFlag = try flag(.is_dm) else { throw ZooError.invalidResponse }
        is_private = privateFlag
        is_dm = dmFlag
        encrypted = try flag(.encrypted)
    }
}

struct ChannelMembership: Decodable, Sendable {
    let channel_id: String
    let user_id: String
}

struct Message: Codable, Identifiable, Hashable, Sendable {
    let id: String
    var user_id: String
    var channel_id: String
    var content: String
    var created_at: String
    var updated_at: String?
    var deleted_at: String?
    var parent_id: String?
    var card: String?
    var display_data: MessageDisplayData?

    var date: Date? { ZooDate.parse(created_at) }
}

enum ZooDate {
    static func parse(_ value: String) -> Date? {
        var normalized = value.replacingOccurrences(of: " ", with: "T")
        if !normalized.hasSuffix("Z"), !normalized.dropFirst(10).contains("+"), !normalized.dropFirst(10).contains("-") {
            normalized += "Z"
        }
        return (try? Date(normalized, strategy: Date.ISO8601FormatStyle(includingFractionalSeconds: true)))
            ?? (try? Date(normalized, strategy: .iso8601))
    }
}

struct Envelope<T: Decodable & Sendable>: Decodable, Sendable {
    let data: T
    let txid: String?
}

enum JSONValue: Codable, Equatable, Sendable {
    case string(String), number(Double), bool(Bool), null
    case object([String: JSONValue]), array([JSONValue])

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let x = try? value.decode(Bool.self) { self = .bool(x) }
        else if let x = try? value.decode(String.self) { self = .string(x) }
        else if let x = try? value.decode(Double.self) { self = .number(x) }
        else if let x = try? value.decode([String: JSONValue].self) { self = .object(x) }
        else { self = .array(try value.decode([JSONValue].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .string(let x): try value.encode(x)
        case .number(let x): try value.encode(x)
        case .bool(let x): try value.encode(x)
        case .null: try value.encodeNil()
        case .object(let x): try value.encode(x)
        case .array(let x): try value.encode(x)
        }
    }
}

enum ZooError: LocalizedError {
    case invalidConfiguration, invalidResponse, expiredSession, rejected(Int, String), auth(String)

    var errorDescription: String? {
        switch self {
        case .invalidConfiguration: "接続設定を確認してください。サーバーと認証先には HTTPS が必要です。"
        case .invalidResponse: "サーバーの応答を読み取れませんでした。再接続してください。"
        case .expiredSession: "ログインの有効期限が切れました。もう一度ログインしてください。"
        case .rejected(let status, let detail): "操作に失敗しました（\(status)）。\(detail)"
        case .auth(let reason): reason
        }
    }
}
