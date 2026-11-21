import Foundation

enum WatchAction: String, Codable, Sendable {
    case refresh, answer, decline, end, setMuted
}

struct WatchRequest: Codable, Sendable {
    var version = 1
    var id = UUID()
    let action: WatchAction
    var sessionID: UUID?
    var callID: UUID?
    var muted: Bool?
}

struct WatchCall: Codable, Equatable, Sendable {
    enum Phase: String, Codable, Sendable { case incoming, connecting, connected }
    let id: UUID
    let title: String
    let phase: Phase
    let status: String
    let muted: Bool
    let canSetMuted: Bool
    var expiresAt: Date?

    func allows(_ action: WatchAction, at date: Date = .now) -> Bool {
        switch action {
        case .refresh: true
        case .answer, .decline: phase == .incoming && expiresAt.map { $0 > date } == true
        case .end: phase != .incoming
        case .setMuted: phase == .connected && canSetMuted
        }
    }
}

struct WatchNotice: Codable, Equatable, Identifiable, Sendable {
    let id: String
    let title: String
    let detail: String
    let date: Date
    let unread: Bool
}

struct WatchSnapshot: Codable, Equatable, Sendable {
    var sessionID: UUID?
    var call: WatchCall?
    var notices: [WatchNotice] = []
    var unreadCount = 0
    var noticesLoading = false
    var noticeError: String?
}

struct WatchReply: Codable, Sendable {
    var version = 1
    var createdAt = Date()
    let snapshot: WatchSnapshot
    var error: String?
}
