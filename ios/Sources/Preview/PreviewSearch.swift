#if DEBUG
import Foundation

/// 検索画面の各状態を、語で切り替えて確かめるためのプレビュー応答。
/// 「失敗」(fail)→ 通信失敗、「なし」(none)→ 0件、「古い」(older)→ 0件で古い期間が残る、「長い」(long)→ 長い日本語。
/// 返信の結果は `--uitesting-thread-directory` の返信を指す。
extension PreviewChatClient {
    func previewSearch(q: String, type: GlobalSearchKind, cursor: String?) async throws -> SearchPage {
        if q.contains("失敗") || q.contains("fail") { throw ZooError.invalidResponse }
        if q.contains("なし") || q.contains("none") { return try page(items: [], next: nil, until: "2025-12") }
        if q.contains("古い") || q.contains("older") {
            if cursor == nil { return try page(items: [], next: #"{"m":24323}"#, until: "2026-05") }
            return try page(items: [message(id: "old", text: "半年前の会議の資料です。", at: "2025-11-03T01:02:03.456")], next: nil, until: "2025-01")
        }
        if q.contains("長い") || q.contains("long") {
            let text = String(repeating: "長い日本語の本文が三行を超えても省略されることを確かめるための文章です。", count: 6)
            return try page(items: [message(id: "long", text: text, at: "2026-11-21T01:02:03.456")], next: nil, until: nil)
        }
        let files = [file(id: "f1", name: "会議の資料.pdf", size: 1_572_864, type: "application/pdf"),
                     file(id: "f2", name: "さくら.png", size: 48_200, type: "image/png")]
        let messages = [
            message(id: "m1", text: "会議の資料を共有します。:stamp: お願いします。", at: "2026-11-21T01:02:03.456"),
            message(id: "raw:77777777-7777-4777-8777-777777777777", text: "返信です。資料を確認しました。", at: "2026-11-20T09:00:00.000", parent: Self.messageID),
        ]
        switch type {
        case .messages: return try page(items: messages, next: nil, until: "2026-04")
        case .files: return try page(items: files, next: nil, until: "2026-04")
        case .all: return try page(items: messages + files, next: cursor == nil ? #"{"m":24323}"# : nil, until: cursor == nil ? "2026-04" : "2025-04")
        }
    }

    private func page(items: [[String: Any]], next: String?, until: String?) throws -> SearchPage {
        let body: [String: Any] = ["items": items, "next_cursor": next.map { $0 as Any } ?? NSNull(), "searched_until": until.map { $0 as Any } ?? NSNull()]
        return try JSONDecoder().decode(SearchPage.self, from: JSONSerialization.data(withJSONObject: body))
    }

    private func base(id: String, at: String) -> [String: Any] {
        let unjoined = ProcessInfo.processInfo.arguments.contains("--uitesting-unjoined-search")
        return ["id": id.hasPrefix("raw:") ? String(id.dropFirst(4)) : "preview-\(id)",
         "channel_id": unjoined ? "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa" : Self.channelID,
         "channel_name": unjoined ? "雑談" : "general", "is_dm": false, "user_id": "44444444-4444-4444-8444-444444444444",
         "author": ["id": "44444444-4444-4444-8444-444444444444", "user_name": "sakura", "display_name": "さくら", "is_bot": false],
         "emojis": ["stamp": ["url": "https://ui.test.invalid/stamp.png"]], "created_at": at]
    }

    private func message(id: String, text: String, at: String, parent: String? = nil) -> [String: Any] {
        var item = base(id: id, at: at)
        item["kind"] = "message"
        item["snippet"] = text
        item["parent_id"] = parent
        return item
    }

    private func file(id: String, name: String, size: Int, type: String) -> [String: Any] {
        var item = base(id: id, at: "2026-11-19T05:00:00.000")
        item["kind"] = "file"
        item["message_id"] = Self.messageID
        item["file_name"] = name
        item["content_type"] = type
        item["file_size"] = size
        item["is_sensitive"] = false
        return item
    }
}
#endif
