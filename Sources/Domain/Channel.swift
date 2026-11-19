import Foundation

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
