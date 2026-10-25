import Foundation

enum SyncPhase: Equatable {
    case idle
    case loading
    case resyncing
    case ready
    case failed(String)
    case accessDenied

    var isLoading: Bool { self == .loading || self == .resyncing }
    var failure: String? {
        switch self {
        case .failed(let message): message
        case .accessDenied: "チャンネルへのアクセスを確認できません。"
        default: nil
        }
    }
}

enum MessagePolicy {
    static func validate(_ text: String) throws {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              text.unicodeScalars.count <= 40_000 else {
            throw ZooError.auth("メッセージは1〜40,000文字で入力してください。")
        }
    }
}

enum ChannelPolicy {
    static func canManage(_ channel: Channel, user: Person?) -> Bool {
        !channel.is_dm && (channel.created_user_id == user?.id || user?.is_admin == true)
    }

    static func canInvite(_ channel: Channel, user: Person?, joined: Bool) -> Bool {
        !channel.is_dm && (canManage(channel, user: user) || (!channel.is_private && joined))
    }
}
