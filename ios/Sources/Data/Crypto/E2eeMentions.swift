import Foundation

/// 暗号化チャンネルの送信に付ける、メンション（docs/e2ee.md の「メッセージ」の `mentions`）。サーバーは本文を読めないので、
/// 平文チャンネルでサーバーが本文から作るのと同じ規則（`backend/src/notifications.rs` の `extract_mentions`）で、送信側が作る。
struct EncryptedMentions: Equatable, Sendable {
    var userIDs: [String] = []
    var channel = false
    var here = false

    private static let pattern = try! NSRegularExpression(pattern: #"(?:^|[\s(\[{<（「『"'。、])(@[a-zA-Z0-9._-]+)"#)
    private static let punctuation = CharacterSet(charactersIn: ".,!?:;。、！？")

    /// `userIDForName` は、ユーザー名（大文字小文字は区別する）から、チャンネルのメンバーでなくてもよいユーザー ID を引く
    /// （メンバーでない ID は、サーバーが黙って無視する）。ユーザー名は、語のまま、なければ末尾の句読点を外して引く。
    static func extract(from content: String, userIDForName: (String) -> String?) -> EncryptedMentions {
        var ids = Set<String>()
        var result = EncryptedMentions()
        let text = content as NSString
        for match in pattern.matches(in: content, range: NSRange(location: 0, length: text.length)) {
            let raw = String(text.substring(with: match.range(at: 1)).dropFirst())
            var trimmed = raw
            while let last = trimmed.unicodeScalars.last, punctuation.contains(last) { trimmed.unicodeScalars.removeLast() }
            if trimmed.lowercased() == "channel" { result.channel = true }
            else if trimmed.lowercased() == "here" { result.here = true }
            else if let id = userIDForName(raw) ?? userIDForName(trimmed) { ids.insert(id) }
        }
        // サーバーの上限は 200 件。
        result.userIDs = Array(ids.sorted().prefix(200))
        return result
    }
}
