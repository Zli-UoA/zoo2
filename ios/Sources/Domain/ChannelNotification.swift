import Foundation

enum ChannelNotificationLevel: String, Codable, CaseIterable, Sendable {
    case all, mentions, mute

    var title: String {
        switch self {
        case .all: "すべてのメッセージ"
        case .mentions: "メンション・返信など"
        case .mute: "ミュート（メンションを除く）"
        }
    }
}

struct ChannelNotificationSetting: Codable, Equatable, Sendable {
    let user_id: String
    let channel_id: String
    let notification_level: ChannelNotificationLevel
}

struct ChannelNotificationSettings: Codable, Sendable {
    let level: ChannelNotificationLevel
}

struct ChannelNotificationSnapshot: Sendable {
    let rows: [ChannelNotificationSetting]?
    var transactionIDs: Set<UInt64> = []
    var freshSnapshot = false
}
