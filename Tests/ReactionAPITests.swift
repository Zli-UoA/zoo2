import Foundation
import Testing
@testable import Zoo

@MainActor
@Suite("スタンプの通信", .timeLimit(.minutes(1)))
struct ReactionAPITests {
    @Test("追加はサーバーが返した表記を採用する")
    func additionReturnsStoredSpelling() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let messageID = UUID().uuidString.lowercased()
        harness.stub { request in
            #expect(request.url?.path == "/api/messages/\(messageID)/reactions")
            #expect(request.httpMethod == "POST")
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            let payload = try JSONDecoder().decode([String: String].self, from: StubProtocol.body(request))
            #expect(payload == ["emoji": ":hana:"])
            let body = #"{"data":{"id":"r1","message_id":"\#(messageID)","user_id":"u","emoji":":sakura:","created_at":"2026-11-18T09:00:00Z"},"txid":null}"#
            return (200, [:], Data(body.utf8))
        }
        let reaction = try await harness.client.addReaction(messageID: messageID, emoji: ":hana:")
        #expect(reaction == MessageReaction(id: "r1", message_id: messageID, user_id: "u", emoji: ":sakura:"))
    }

    @Test("取り消しは元の Unicode やプラス記号を含む表記をクエリに保持する", arguments: ["👩🏽‍💻", "apub:" + String(repeating: "a", count: 64) + "::+1:"])
    func deletionPreservesEmoji(emoji: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let messageID = UUID().uuidString.lowercased()
        harness.stub { request in
            let url = try #require(request.url)
            #expect(url.path == "/api/messages/\(messageID)/reactions")
            #expect(request.httpMethod == "DELETE")
            let parts = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false))
            #expect(parts.queryItems == [URLQueryItem(name: "emoji", value: emoji)])
            #expect(parts.percentEncodedQuery?.contains("+") == false)
            return (200, [:], Data())
        }
        try await harness.client.removeReaction(messageID: messageID, emoji: emoji)
    }

    @Test("カタログの相対画像 URL は接続先から解決する")
    func resolvesCatalogImages() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.url?.path == "/api/emojis")
            return (200, [:], Data(#"[{"name":"zoo","category":"動物園","aliases":["どうぶつ"],"url":"/api/emoji-images/zoo?size=64"}]"#.utf8))
        }
        let emoji = try #require(try await harness.client.emojis().first)
        #expect(emoji.aliases == ["どうぶつ"])
        #expect(emoji.imageURL?.absoluteString == "https://api.test.invalid/api/emoji-images/zoo?size=64")
    }

    @Test("アカウントスタンプの専用名簿を読み、画像なしや相対画像にも対応する")
    func fetchesAccountDirectory() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/api/emoji-accounts")
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            let body = #"{"accounts":[{"username":"sakura","display_name":"さくら","avatar_url":"/images/sakura.png"},{"username":"hana","display_name":"","avatar_url":null},{"username":"zoo","display_name":"動物園"}]}"#
            return (200, [:], Data(body.utf8))
        }
        let accounts = try await harness.client.emojiAccounts()
        #expect(accounts.map(\.id) == ["sakura", "hana", "zoo"])
        #expect(accounts.map(\.display_name) == ["さくら", "", "動物園"])
        #expect(accounts.map { $0.avatarURL?.absoluteString } == ["https://api.test.invalid/images/sakura.png", nil, nil])
    }

    @Test("旧形式のチャンネルでは専用 shape からスタンプを読む")
    func fallbackStreamUsesChannelFilter() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let channelID = UUID().uuidString.lowercased()
        harness.stub { request in
            let url = try #require(request.url)
            #expect(url.path == "/api/shapes/message_reactions")
            let items = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems)
            #expect(items.contains(URLQueryItem(name: "channel_id", value: channelID)))
            let events = #"[{"key":"r1","headers":{"operation":"insert"},"value":{"id":"r1","message_id":"m","user_id":"u","emoji":"👍"}},{"headers":{"control":"up-to-date"}}]"#
            return (200, ["electric-offset": "0_1", "electric-handle": "shape"], Data(events.utf8))
        }
        let snapshots = AsyncStream<[MessageReaction]>.makeStream()
        let stream = Task {
            defer { snapshots.continuation.finish() }
            try await harness.client.observeReactions(channelID: channelID) { rows in
                if let rows { snapshots.continuation.yield(rows) }
            }
        }
        var iterator = snapshots.stream.makeAsyncIterator()
        let rows = await iterator.next()
        #expect(rows == [MessageReaction(id: "r1", message_id: "m", user_id: "u", emoji: "👍")])
        stream.cancel()
        await #expect(throws: CancellationError.self) { try await stream.value }
    }

    @Test("メッセージに含まれるスタンプはオブジェクトと JSON 文字列の両方で読める", arguments: [false, true])
    func decodesEmbeddedDisplayData(stringified: Bool) throws {
        let raw = #"{"reactions":[{"id":"remote","message_id":"m","user_id":"actor","emoji":":hana:"}],"emojis":{"hana":{"url":"/icons/a","width":24}}}"#
        let embedded = stringified ? try JSONEncoder().encode(raw) : Data(raw.utf8)
        let message = #"{"id":"m","user_id":"u","channel_id":"c","content":"本文","created_at":"2026-11-18","display_data":\#(String(decoding: embedded, as: UTF8.self))}"#
        let decoded = try JSONDecoder().decode(Message.self, from: Data(message.utf8))
        #expect(decoded.display_data?.reactions?.first?.id == "remote")
        #expect(decoded.display_data?.emojis["hana"]?.imageURL(relativeTo: "https://api.test.invalid")?.path == "/icons/a")
    }
}
