import Foundation

struct Person: Codable, Identifiable, Hashable, Sendable {
    let id: String
    var user_name: String
    var display_name: String
    var is_bot: Bool = false
    var is_admin: Bool? = nil
}

struct Channel: Codable, Identifiable, Hashable, Sendable {
    let id: String
    var channel_name: String
    var description: String?
    var created_user_id: String
    var is_private: Bool
    var is_dm: Bool
    var archive_at: String?
    var deleted_at: String?
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
}

struct Reaction: Codable, Identifiable, Hashable, Sendable {
    let id: String
    let message_id: String
    let user_id: String
    let emoji: String
}

struct Membership: Codable, Sendable {
    let user_id: String
    let channel_id: String
}

struct Activity: Codable, Identifiable, Hashable, Sendable {
    let id: String
    let actor_id: String
    let channel_id: String
    let message_id: String
    let notification_type: String
    var is_read: Bool
    let created_at: String

    var summary: String {
        switch notification_type {
        case "mention": "あなたにメンションしました"
        case "reply": "スレッドに返信しました"
        case "reaction": "リアクションしました"
        case "dm": "メッセージを送りました"
        default: "新しい通知"
        }
    }
}

struct ReadState: Decodable, Sendable {
    let channel_id: String
    let has_unread: Bool
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

enum ZooDate {
    static func parse(_ raw: String) -> Date? {
        var text = raw.replacingOccurrences(of: " ", with: "T")
        let suffix = text.dropFirst(10)
        if !text.hasSuffix("Z") && !suffix.contains("+") && !suffix.contains("-") { text += "Z" }
        return (try? Date.ISO8601FormatStyle(includingFractionalSeconds: true).parse(text)) ??
            (try? Date.ISO8601FormatStyle().parse(text))
    }

    static func now() -> String { ISO8601DateFormatter().string(from: Date()) }
}
