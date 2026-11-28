import Foundation

enum SearchPeriod: String, CaseIterable, Identifiable, Sendable {
    case anytime, last7, last30, last365, custom

    var id: String { rawValue }
    var title: String {
        switch self {
        case .anytime: "指定なし"
        case .last7: "過去 7 日間"
        case .last30: "過去 30 日間"
        case .last365: "過去 1 年間"
        case .custom: "日付を指定"
        }
    }
    var days: Int? {
        switch self {
        case .last7: 7
        case .last30: 30
        case .last365: 365
        default: nil
        }
    }
}

/// サーバーへ送る絞り込み。期間は検索の開始時に決め、続きのページでも同じ値を送る。
struct SearchRequestFilters: Equatable, Sendable {
    var userIDs: [String] = []
    var channelIDs: [String] = []
    var excludeBots = false
    var involvingMe = false
    var after: String?
    var before: String?

    var queryItems: [String: String] {
        var query: [String: String] = [:]
        if !userIDs.isEmpty { query["user_id"] = userIDs.joined(separator: ",") }
        if !channelIDs.isEmpty { query["channel_id"] = channelIDs.joined(separator: ",") }
        if excludeBots { query["exclude_bots"] = "true" }
        if involvingMe { query["involving_me"] = "true" }
        if let after { query["after"] = after }
        if let before { query["before"] = before }
        return query
    }
}

/// 画面で選ぶ絞り込み（Web の検索と同じ項目）。
struct SearchFilters: Equatable, Sendable {
    /// サーバーの `MAX_FILTER_IDS`。
    static let maximumSelection = 20

    var senders: [String] = []
    var channels: [String] = []
    var period: SearchPeriod = .anytime
    var customAfter = Date()
    var customEnd = Date()
    var involvingMe = false
    var excludeBots = false

    var isActive: Bool {
        !senders.isEmpty || !channels.isEmpty || period != .anytime || involvingMe || excludeBots
    }

    /// 開始日が終了日より後だと検索できない。
    func hasReversedDays(calendar: Calendar = .current) -> Bool {
        period == .custom && calendar.startOfDay(for: customAfter) > calendar.startOfDay(for: customEnd)
    }

    func resolved(now: Date = Date(), calendar: Calendar = .current) -> SearchRequestFilters {
        var request = SearchRequestFilters(userIDs: Array(senders.prefix(Self.maximumSelection)),
                                           channelIDs: Array(channels.prefix(Self.maximumSelection)),
                                           excludeBots: excludeBots, involvingMe: involvingMe)
        let today = calendar.startOfDay(for: now)
        if let days = period.days, let start = calendar.date(byAdding: .day, value: -(days - 1), to: today) {
            request.after = Self.timestamp(start, calendar: calendar)
        } else if period == .custom, !hasReversedDays(calendar: calendar) {
            request.after = Self.timestamp(calendar.startOfDay(for: customAfter), calendar: calendar)
            // 終了日はその日を含む。API の `before` はその翌日の 0 時。
            if let next = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: customEnd)) {
                request.before = Self.timestamp(next, calendar: calendar)
            }
        }
        return request
    }

    /// 端末の暦の日の 0 時を、オフセット付きの RFC 3339 で表す（UTC の日付で区切ると、日本の 0〜9 時が前日に入るため）。
    static func timestamp(_ date: Date, calendar: Calendar) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        formatter.timeZone = calendar.timeZone
        return formatter.string(from: date)
    }
}
