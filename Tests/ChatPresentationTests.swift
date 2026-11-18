import Foundation
import Synchronization
import Testing
@testable import Zoo

@Suite("日付・未読・メンション")
struct ChatPresentationTests {
    private func message(_ id: String, date: String, user: String = "other", parent: String? = nil) -> Message {
        Message(id: id, user_id: user, channel_id: "channel", content: "本文", created_at: date, parent_id: parent)
    }

    @Test("未読境界は時刻より後の他人のルート投稿に置く")
    func unread() {
        let messages = [message("old", date: "2026-11-18T09:00:00Z"),
                        message("own", date: "2026-11-18T09:01:00Z", user: "me"),
                        message("reply", date: "2026-11-18T09:02:00Z", parent: "old"),
                        message("new", date: "2026-11-18T09:03:00Z")]
        let state = ChannelReadState(channel_id: "channel", has_unread: true, last_read_at: messages[0].created_at)
        #expect(MessageTimeline.firstUnread(in: messages, state: state, userID: "me") == "new")
        #expect(MessageTimeline.firstUnread(in: messages, state: nil, userID: "me") == nil)
        #expect(MessageTimeline.firstUnread(in: messages, state: ChannelReadState(channel_id: "channel", has_unread: false, last_read_at: nil), userID: "me") == nil)
    }

    @Test("日付区切りは端末の暦とタイムゾーンで判断する")
    func dayBoundary() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try #require(TimeZone(identifier: "Asia/Tokyo"))
        let before = message("before", date: "2026-11-18T14:59:59Z")
        let after = message("after", date: "2026-11-18T15:00:00Z")
        #expect(MessageTimeline.startsDay(after, after: before, calendar: calendar))
        #expect(!MessageTimeline.startsDay(after, after: after, calendar: calendar))
        #expect(MessageTimeline.startsDay(before, after: nil, calendar: calendar))
    }

    @Test("本文のメンションは既知名だけを強調し、メール・コード・URL・連合名を保つ")
    func mentions() {
        func parse(_ text: String) -> [MessageContent.Part] {
            MessageContent.parse(text, mention: { ["you", "sakura", "channel"].contains($0) ? $0 == "you" : nil }) { _ in nil }
        }
        #expect(parse("@you、@sakura. @missing") == [.mention("@you", isSelf: true), .text("、"), .mention("@sakura", isSelf: false), .text(". @missing")])
        for text in ["mail@you", "`@you`", "```@you```", ":@you:", "@you@example.org", "@sakurax"] {
            #expect(!parse(text).contains { if case .mention = $0 { return true }; return false })
        }
    }

    @Test("入力途中の候補はカーソル位置・全角文字・コード境界を扱う")
    func suggestions() throws {
        let text = "😀相談（@sakura 後半"
        let caret = try #require(text.range(of: "sak")) .upperBound
        let query = try #require(MentionInput.query(in: text, caret: caret))
        #expect(query.text == "sak")
        #expect(String(text[query.range]) == "@sakura")
        for value in ["mail@sa", "`@sa", "```\n@sa", "https://example.org/@sa"] {
            #expect(MentionInput.query(in: value, caret: value.endIndex) == nil)
        }
        let fullwidth = "@ｓａ"
        #expect(MentionInput.query(in: fullwidth, caret: fullwidth.endIndex)?.text == "sa")
        let people = [Person(id: "a", user_name: "sakura", display_name: "さくら"),
                      Person(id: "b", user_name: "saki", display_name: "停止済み", is_deactivated: true)]
        #expect(MentionInput.candidates(people: people, query: "さく").map(\.username) == ["sakura"])
        #expect(MentionInput.candidates(people: people, query: "sa").map(\.username) == ["sakura"])
    }

    @Test("古い選択位置と絵文字の途中は入力候補に使わない")
    func staleCaret() {
        let previous = "@sakura"
        #expect(MentionInput.query(in: "", caret: previous.endIndex) == nil)
        #expect(MentionInput.query(in: "@", caret: previous.endIndex) == nil)
        let ascii = "ab"
        #expect(MentionInput.query(in: "😀", caret: ascii.index(after: ascii.startIndex)) == nil)
    }

    @Test("Electricの文字列型の寸法とセンシティブ属性を復号する")
    func attachmentShape() throws {
        let data = Data(#"{"id":"a","message_id":"m","file_name":"動画.mp4","content_type":"video/mp4","width":"640","height":"480","is_sensitive":"f"}"#.utf8)
        let attachment = try JSONDecoder().decode(MessageAttachment.self, from: data)
        #expect(attachment.width == 640 && attachment.height == 480)
        #expect(attachment.is_sensitive == false && attachment.isVideo)
        #expect(attachment.message_id == "m")
    }

    @Test("添付を JSONB の文字列・オブジェクトから復号する", arguments: [false, true])
    func attachments(string: Bool) throws {
        let raw = #"{"attachments":[{"id":"a","file_name":"画像.png","content_type":"image/png","width":640,"is_sensitive":true}]}"#
        let data = string ? try JSONEncoder().encode(raw) : Data(raw.utf8)
        let result = try JSONDecoder().decode(MessageDisplayData.self, from: data)
        let attachment = try #require(result.attachments.first)
        #expect(attachment.isImage)
        #expect(attachment.is_sensitive == true)
    }
}

@MainActor @Suite("チャット表示のAPI契約")
struct ChatPresentationAPITests {
    private let channel = Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "一般", is_private: false, is_dm: false)

    @Test("ピン追加・解除は版を送り、Envelope内の最新一覧を採用する")
    func pinWrites() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let id = channel.id
        harness.stub { request in
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            if request.httpMethod == "POST" {
                #expect(request.url?.path == "/api/channel-pins")
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body["channel_id"] == .string(id))
                #expect(body["expected_version"] == .number(7))
                return (200, [:], Data("{\"data\":{\"pins\":[{\"channel_id\":\"\(id)\",\"position\":0}],\"version\":8}}".utf8))
            }
            #expect(request.httpMethod == "DELETE")
            #expect(request.url?.path == "/api/channel-pins/\(id)")
            #expect(URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems?.first == URLQueryItem(name: "expected_version", value: "8"))
            return (200, [:], Data(#"{"data":{"pins":[],"version":9}}"#.utf8))
        }
        #expect(try await harness.client.setChannelPinned(id: id, pinned: true, expectedVersion: 7).pins.first?.channel_id == id)
        #expect(try await harness.client.setChannelPinned(id: id, pinned: false, expectedVersion: 8).pins.isEmpty == true)
    }

    @Test("別端末との競合は最新の版で再試行し、他のピンを保持する")
    func pinConflict() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let requests = Mutex(0)
        let id = channel.id
        harness.stub { request in
            let index = requests.withLock { $0 += 1; return $0 }
            switch index {
            case 1:
                #expect(request.httpMethod == "GET")
                return (200, [:], Data(#"{"pins":[],"version":1}"#.utf8))
            case 2:
                #expect(request.httpMethod == "POST")
                return (409, [:], Data(#"{"error":"CHANNEL_PINS_CONFLICT"}"#.utf8))
            case 3:
                #expect(request.httpMethod == "GET")
                return (200, [:], Data(#"{"pins":[{"channel_id":"other","position":0}],"version":2}"#.utf8))
            case 4:
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(request.httpMethod == "POST")
                #expect(body["expected_version"] == .number(2))
                return (200, [:], Data("{\"data\":{\"pins\":[{\"channel_id\":\"other\",\"position\":0},{\"channel_id\":\"\(id)\",\"position\":1}],\"version\":3}}".utf8))
            default:
                Issue.record("予期しない再試行")
                return (500, [:], Data())
            }
        }
        await workspace.setChannelPinned(channel, pinned: true)
        #expect(workspace.pinnedChannelIDs == ["other", id])
        #expect(workspace.pinUpdateError == nil)
        #expect(!workspace.updatingPin)
        #expect(requests.withLock { $0 } == 4)
        harness.stub { _ in (200, [:], Data(#"{"pins":[],"version":1}"#.utf8)) }
        await workspace.loadPins()
        #expect(workspace.pinnedChannelIDs == ["other", id])
    }

    @Test("ピン保存失敗は確定済みの一覧を保ち、再操作できる", arguments: [400, 404, 409, 500])
    func pinFailure(status: Int) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let writes = Mutex(0)
        let id = channel.id
        harness.stub { request in
            if request.httpMethod == "GET" {
                return (200, [:], Data("{\"pins\":[{\"channel_id\":\"\(id)\",\"position\":0}],\"version\":1}".utf8))
            }
            writes.withLock { $0 += 1 }
            return (status, [:], Data(#"{"error":"保存失敗"}"#.utf8))
        }
        await workspace.setChannelPinned(channel, pinned: false)
        #expect(workspace.pinnedChannelIDs == [id])
        #expect(workspace.pinUpdateError != nil)
        #expect(!workspace.updatingPin)
        #expect(writes.withLock { $0 } == (status == 409 ? 2 : 1))
    }

    @Test("ピンの非Envelope応答と署名URLのEnvelope応答を使い分ける")
    func contracts() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            switch request.url?.path {
            case "/api/channel-pins": return (200, [:], Data(#"{"pins":[{"channel_id":"c","position":2}],"version":3}"#.utf8))
            case "/api/attachments/11111111-1111-4111-8111-111111111111/url":
                return (200, [:], Data(#"{"data":{"download_url":"https://files.example.org/photo.png?signature=temporary"}}"#.utf8))
            default: Issue.record("予期しないAPI呼び出し"); return (404, [:], Data())
            }
        }
        #expect(try await harness.client.channelPins().pins.first?.channel_id == "c")
        #expect(try await harness.client.attachmentURL(id: "11111111-1111-4111-8111-111111111111").host == "files.example.org")
    }

    @Test("不正な署名URLを表示層に渡さない", arguments: ["http://files.example.org/a", "https://user:pass@files.example.org/a", "file:///tmp/a"])
    func unsafeURLs(_ url: String) throws {
        let download = try JSONDecoder().decode(AttachmentDownload.self, from: JSONEncoder().encode(["download_url": url]))
        #expect(download.url == nil)
    }
}
