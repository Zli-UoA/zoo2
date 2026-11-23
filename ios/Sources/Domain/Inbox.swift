import Foundation

struct InboxReaction: Codable, Equatable, Sendable {
    let user_id: String
    let emoji: String
}

struct InboxContext: Decodable, Equatable, Sendable {
    struct Preview: Codable, Equatable, Sendable { let content: String }
    var message: Preview?
    var reactions: [InboxReaction]
    var content: String? { message?.content }
}

struct ChannelNotice: Decodable, Equatable, Sendable {
    struct Display: Decodable, Equatable, Sendable {
        struct Actor: Decodable, Equatable, Sendable {
            let display_name: String?
            let user_name: String?
            let avatar_url: String?
        }
        struct Place: Decodable, Equatable, Sendable { let channel_name: String }
        let channel: Place?
        let actor: Actor?
        let message: InboxContext.Preview?
        let reactions: [InboxReaction]?
    }

    let id: String
    let user_id: String
    let actor_id: String
    let channel_id: String
    let message_id: String?
    let notification_type: String
    let created_at: String
    let is_read: Bool
    let display: Display?

    enum CodingKeys: String, CodingKey {
        case id, user_id, actor_id, channel_id, message_id, notification_type, created_at, is_read, display_data
    }

}

extension ChannelNotice {
    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        user_id = try values.decode(String.self, forKey: .user_id)
        actor_id = try values.decode(String.self, forKey: .actor_id)
        channel_id = try values.decode(String.self, forKey: .channel_id)
        message_id = try values.decodeIfPresent(String.self, forKey: .message_id)
        notification_type = try values.decode(String.self, forKey: .notification_type)
        created_at = try values.decode(String.self, forKey: .created_at)
        switch try values.decode(JSONValue.self, forKey: .is_read) {
        case .bool(let value): is_read = value
        case .string("true"), .string("t"): is_read = true
        case .string("false"), .string("f"): is_read = false
        default: throw ZooError.invalidResponse
        }
        if let raw = try? values.decode(String.self, forKey: .display_data) {
            display = try? JSONDecoder().decode(Display.self, from: Data(raw.utf8))
        } else {
            display = try? values.decode(Display.self, forKey: .display_data)
        }
    }
}

struct FederatedNotice: Codable, Sendable {
    struct Avatar: Codable, Sendable { let image_id: String? }
    struct Emoji: Codable, Sendable {
        let image_id: String?
        let name: String
    }
    let id: String
    let actor_id: String
    let display_name: String
    let avatar: Avatar?
    let kind: String
    let activity_id: String
    let post_id: String?
    let content: String?
    let created_at: String
    var is_read: Bool
    var pending: Bool
    let reaction: String?
    var emoji: Emoji? = nil

    var displayReaction: String? {
        if let emoji, let id = emoji.image_id, id.count == 64, id.allSatisfy(\.isHexDigit) {
            return "apub:\(id):\(emoji.name)"
        }
        return reaction ?? emoji?.name
    }
}

struct FederatedNoticePage: Codable, Sendable {
    let items: [FederatedNotice]
    let unread_count: Int
    let next_cursor: String?
}

struct InboxEntry: Identifiable, Equatable, Sendable {
    let id: String
    let kind: String
    let createdAt: Date
    var unreadIDs: [String]
    var isUnread: Bool { !unreadIDs.isEmpty }
    var actorIDs: [String]
    let actorName: String?
    let avatarURL: URL?
    let channelID: String?
    let channelName: String?
    let messageID: String?
    let content: String?
    let reaction: String?
    let pendingFollow: Bool
    let noticeID: String
    let activityID: String?
    let joinedContext: InboxContext?
}

struct InboxFeed {
    var entries: [InboxEntry]
    var unreadCount: Int

    init(channels: [ChannelNotice], federated: [FederatedNotice], federatedUnread: Int, more: Bool,
         acknowledged: Set<String>, server: String) {
        var grouped: [String: InboxEntry] = [:]
        var mirrors: [String: (key: String, unread: Bool)] = [:]
        var unread = federatedUnread
        let sorted = channels.map { ($0, ZooDate.parse($0.created_at) ?? .distantPast) }
            .sorted { ($0.1, $0.0.id) > ($1.1, $1.0.id) }
        for (row, date) in sorted {
            let key: String
            if row.notification_type == "reaction", let message = row.message_id {
                key = "reaction:\(row.channel_id):\(message)"
            } else {
                key = "channel:\(row.id)"
            }
            let isUnread = !row.is_read && !acknowledged.contains(row.id)
            let unreadIDs = isUnread ? [row.id] : []
            if isUnread { unread += 1 }
            if grouped[key] != nil {
                grouped[key]?.unreadIDs.append(contentsOf: unreadIDs)
                if grouped[key]?.actorIDs.contains(row.actor_id) == false { grouped[key]?.actorIDs.append(row.actor_id) }
            } else {
                let display = row.display
                let name = display?.actor?.display_name
                grouped[key] = InboxEntry(id: key, kind: row.notification_type, createdAt: date, unreadIDs: unreadIDs,
                                          actorIDs: [row.actor_id], actorName: name?.isEmpty == false ? name : display?.actor?.user_name,
                                          avatarURL: DisplayEmoji.assetURL(display?.actor?.avatar_url, relativeTo: server),
                                          channelID: row.channel_id, channelName: display?.channel?.channel_name,
                                          messageID: row.message_id, content: display?.message?.content,
                                          reaction: nil, pendingFollow: false, noticeID: row.id, activityID: nil,
                                          joinedContext: display.map { InboxContext(message: $0.message, reactions: $0.reactions ?? []) })
            }
            if let message = row.message_id, ["reply", "mention", "reaction"].contains(row.notification_type) {
                let kind = row.notification_type == "reaction" ? "like" : "reply"
                let mirrorID = "\(kind):\(row.actor_id):\(message)"
                let previous = mirrors[mirrorID]
                mirrors[mirrorID] = (previous?.key ?? key, isUnread || previous?.unread == true)
            }
        }
        var oldest: Date?
        for row in federated {
            let isUnread = !row.is_read && !acknowledged.contains(row.id)
            let date = ZooDate.parse(row.created_at) ?? .distantPast
            oldest = min(date, oldest ?? date)
            if row.actor_id.hasPrefix("local:"), let post = row.post_id,
               let mirror = mirrors["\(row.kind):\(row.actor_id.dropFirst(6)):\(post)"] {
                if isUnread {
                    grouped[mirror.key]?.unreadIDs.append(row.id)
                    if mirror.unread { unread -= 1 }
                }
                continue
            }
            let key = "activitypub:\(row.id)"
            let imageID = row.avatar?.image_id
            let imageURL = imageID.flatMap { id in
                id.count == 64 && id.allSatisfy(\.isHexDigit)
                    ? DisplayEmoji.assetURL("/api/activitypub/emoji-images/\(id)", relativeTo: server) : nil
            }
            grouped[key] = InboxEntry(id: key, kind: row.kind, createdAt: date, unreadIDs: isUnread ? [row.id] : [],
                                      actorIDs: [row.actor_id], actorName: row.display_name, avatarURL: imageURL,
                                      channelID: nil, channelName: nil, messageID: row.post_id, content: row.content,
                                      reaction: row.displayReaction, pendingFollow: row.pending, noticeID: row.id,
                                      activityID: row.activity_id, joinedContext: nil)
        }
        entries = grouped.values.filter { !more || $0.createdAt >= (oldest ?? .distantPast) }
            .sorted { ($0.createdAt, $0.id) > ($1.createdAt, $1.id) }
        unreadCount = max(0, unread)
    }
}
