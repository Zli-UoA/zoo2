import Foundation

struct MessageScrollState: Equatable {
    let followingLatest: Bool
    let showsLatestButton: Bool

    init(contentHeight: CGFloat, offsetY: CGFloat, topInset: CGFloat, viewportHeight: CGFloat) {
        // 上側の余白を含む座標差を補正し、最新位置を過去ログの閲覧中と誤判定しない。
        self.init(distanceToBottom: contentHeight - offsetY - topInset - viewportHeight)
    }

    init(distanceToBottom: CGFloat) {
        followingLatest = distanceToBottom <= 80
        // 少し履歴を確認しただけではボタンを重ねず、既読・新着追従の判定は従来どおりに保つ。
        showsLatestButton = distanceToBottom > 240
    }
}

struct ChannelPin: Codable, Sendable, Equatable {
    let channel_id: String
    let position: Int64
}

extension ChannelPin {
    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        channel_id = try values.decode(String.self, forKey: .channel_id)
        if let number = try? values.decode(Int64.self, forKey: .position) { position = number; return }
        // Electricの整数列は文字列、RESTの並び順は数値で届く。
        guard let number = Int64(try values.decode(String.self, forKey: .position)) else { throw ZooError.invalidResponse }
        position = number
    }
}

struct ChannelPins: Codable, Sendable {
    let pins: [ChannelPin]
    let version: Int64
}

struct MessageAttachment: Codable, Identifiable, Hashable, Sendable {
    let id: String
    let file_name: String
    let content_type: String
    var message_id: String? = nil
    var width: Int? = nil
    var height: Int? = nil
    var is_sensitive: Bool? = nil
    // 復号情報はメモリーだけで参照し、APIやキャッシュへ書き出さない。
    var encryptedFileID: String? = nil

    private enum CodingKeys: String, CodingKey {
        case id, file_name, content_type, message_id, width, height, is_sensitive
    }

    var isImage: Bool { content_type.lowercased().hasPrefix("image/") }
    var isVideo: Bool { content_type.lowercased().hasPrefix("video/") }
    var isPDF: Bool {
        let type = content_type.components(separatedBy: ";")[0].trimmingCharacters(in: .whitespaces).lowercased()
        return type == "application/pdf" ||
            (["", "application/octet-stream", "binary/octet-stream"].contains(type) &&
             (file_name as NSString).pathExtension.lowercased() == "pdf")
    }
}

extension MessageAttachment {
    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        file_name = try values.decode(String.self, forKey: .file_name)
        content_type = try values.decode(String.self, forKey: .content_type)
        message_id = try values.decodeIfPresent(String.self, forKey: .message_id)
        func dimension(_ key: CodingKeys) throws -> Int? {
            guard let raw = try values.decodeIfPresent(JSONValue.self, forKey: key) else { return nil }
            switch raw {
            case .number(let value): return Int(exactly: value)
            case .string(let value): return Int(value)
            default: return nil
            }
        }
        width = try dimension(.width); height = try dimension(.height)
        if let raw = try values.decodeIfPresent(JSONValue.self, forKey: .is_sensitive) {
            switch raw {
            case .bool(let value): is_sensitive = value
            case .string("t"), .string("true"): is_sensitive = true
            case .string("f"), .string("false"): is_sensitive = false
            default: is_sensitive = true
            }
        } else { is_sensitive = nil }
    }
}

struct AttachmentDownload: Decodable, Sendable {
    let download_url: String
    var url: URL? { DisplayEmoji.assetURL(download_url) }
}

enum MessageTimeline {
    /// 暗号化チャンネルで、「ここから暗号化されています」の境目を出す位置（読み込んだうちで最初の暗号化したメッセージ
    /// ＝暗号文か `epoch` のある行の添字）。それより上に行がないとき（先頭）や、暗号化したメッセージがないときは `nil`。
    static func encryptionStartIndex(in messages: [Message], channelEncrypted: Bool) -> Int? {
        guard channelEncrypted, let index = messages.firstIndex(where: { $0.ciphertext != nil || $0.epoch != nil }),
              index > 0 else { return nil }
        return index
    }

    static func firstUnread(in messages: [Message], state: ChannelReadState?, userID: String) -> String? {
        guard let state, state.has_unread else { return nil }
        let boundary: Date
        if let raw = state.last_read_at {
            guard let date = ZooDate.parse(raw) else { return nil }
            boundary = date
        } else { boundary = .distantPast }
        return messages.first { $0.parent_id == nil && $0.user_id != userID && ($0.date ?? .distantPast) > boundary }?.id
    }

    static func startsDay(_ message: Message, after previous: Message?, calendar: Calendar = .current) -> Bool {
        guard let date = message.date else { return false }
        guard let previous = previous?.date else { return true }
        return !calendar.isDate(date, inSameDayAs: previous)
    }
}

enum MentionInput {
    struct Query: Equatable {
        let range: Range<String.Index>
        let text: String
    }
    struct Candidate: Identifiable {
        let username: String
        let name: String
        let avatarURL: URL?
        var id: String { username }
    }
    private static let pattern = try! NSRegularExpression(pattern: #"(?:^|[\s(\[{<（「『\"'。、])@([\p{L}\p{M}\p{N}._-]*)\z"#)
    private static let code = try! NSRegularExpression(pattern: #"(?s)```.*?(?:```|$)|`[^`\n]*(?:`|$)"#)

    static func query(in text: String, caret: String.Index) -> Query? {
        guard !text.isEmpty, caret >= text.startIndex, caret <= text.endIndex,
              caret == text.endIndex || text.indices.contains(caret) else { return nil }
        let prefix = String(text[..<caret])
        let full = NSRange(prefix.startIndex..., in: prefix)
        guard let match = pattern.firstMatch(in: prefix, range: full) else { return nil }
        let name = match.range(at: 1)
        let token = NSRange(location: name.location - 1, length: name.length + 1)
        guard !code.matches(in: prefix, range: full).contains(where: { NSIntersectionRange($0.range, token).length > 0 }),
              let start = Range(token, in: text)?.lowerBound,
              let nameRange = Range(name, in: prefix) else { return nil }
        var end = caret
        // カーソルが名前の途中にある場合も、置換後に元の接尾辞を残さない。
        while end < text.endIndex, text[end].unicodeScalars.allSatisfy({ CharacterSet.alphanumerics.union(.init(charactersIn: "._-")).contains($0) }) {
            end = text.index(after: end)
        }
        return Query(range: start..<end, text: String(prefix[nameRange]).precomposedStringWithCompatibilityMapping)
    }

    static func candidates(people: [Person], query: String) -> [Candidate] {
        let broad = [Candidate(username: "channel", name: "チャンネル全員", avatarURL: nil),
                     Candidate(username: "here", name: "オンラインのメンバー", avatarURL: nil)]
        let users = people.filter {
            !$0.is_deactivated && !["channel", "here"].contains($0.user_name.lowercased()) &&
            $0.user_name.range(of: #"^[A-Za-z0-9._-]+$"#, options: .regularExpression) != nil
        }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            .map { Candidate(username: $0.user_name, name: $0.name, avatarURL: $0.avatarURL) }
        return Array((broad + users).filter {
            query.isEmpty || $0.username.localizedStandardContains(query) ||
            $0.name.precomposedStringWithCompatibilityMapping.localizedStandardContains(query)
        }.prefix(8))
    }
}
