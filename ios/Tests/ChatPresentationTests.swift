import Foundation
import Observation
import Synchronization
import Testing
@testable import Zoo

@Suite("最新メッセージへのスクロール")
struct MessageScrollStateTests {
    @Test("実機の上側余白を補正し、末尾と過去ログを区別する", arguments: [
        (611_062.0, 116.0, true, false),
        (610_508.0, 670.0, true, false),
        (610_982.0, 116.0, true, false),
        (610_981.0, 116.0, false, false),
        (610_821.0, 116.0, false, true)
    ])
    func deviceInsets(_ offsetY: Double, _ topInset: Double, _ followsLatest: Bool, _ showsButton: Bool) {
        let state = MessageScrollState(contentHeight: 611_867, offsetY: offsetY, topInset: topInset, viewportHeight: 689)
        #expect(state.followingLatest == followsLatest)
        #expect(state.showsLatestButton == showsButton)
    }

    @Test("末尾との距離に応じて追従とボタン表示を別々に判定する", arguments: [
        (-100.0, true, false), (0.0, true, false), (80.0, true, false),
        (81.0, false, false), (120.0, false, false), (240.0, false, false),
        (241.0, false, true), (1000.0, false, true)
    ])
    func distance(_ distance: Double, _ followsLatest: Bool, _ showsButton: Bool) {
        let state = MessageScrollState(distanceToBottom: CGFloat(distance))
        #expect(state.followingLatest == followsLatest)
        #expect(state.showsLatestButton == showsButton)
    }
}

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
        for value in ["mail@sa", "`@sa", "```\n@sa", "https://example.org/@sa", "@al\n", "@al\r\n"] {
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

    @Test("初回取得の失敗後も再試行中は読み込み表示へ戻り、中断後はエラー表示に戻る",
          .timeLimit(.minutes(1)))
    func directoryShowsRetryProgress() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let workspace = WorkspaceStore(client: harness.client,
                                       user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        harness.stub { _ in (503, [:], Data()) }
        await workspace.refresh()
        #expect(!workspace.hasLoaded && workspace.error != nil)
        #expect(!workspace.directoryLoading)
        let started = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish() }
        harness.stub { _ in
            started.continuation.yield(())
            try await Task.sleep(for: .seconds(60))
            throw CancellationError()
        }
        let retry = Task { await workspace.refresh() }
        defer { retry.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        #expect(!workspace.hasLoaded && workspace.directoryLoading)
        retry.cancel()
        await retry.value
        #expect(!workspace.directoryLoading && workspace.error != nil)
    }

    @Test("通常一覧とピンの到着順によらず、両方がそろってから一覧を表示する", arguments: [false, true])
    func directoryWaitsForPins(pinsFirst: Bool) async {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        #expect(!workspace.directoryReady)
        if pinsFirst { await workspace.loadPins() }
        else { await workspace.refresh() }
        #expect(!workspace.directoryReady)
        if pinsFirst { await workspace.refresh() }
        else { await workspace.loadPins() }
        #expect(workspace.directoryReady)
        #expect(!workspace.channels.isEmpty)
        workspace.revokeChannel(PreviewChatClient.channelID)
        #expect(workspace.directoryReady)
        #expect(!workspace.channels.isEmpty)
    }

    @Test("ピン取得の失敗や初回同期の遅延でも一覧を操作でき、再試行でピンを反映する",
          .timeLimit(.minutes(1)), arguments: [false, true])
    func directoryRemainsAvailableAfterPinsFailure(stalled: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let workspace = WorkspaceStore(client: harness.client,
                                       user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let channel = channel
        harness.stub { request in
            switch request.url?.path {
            case "/api/channels": return (200, [:], try JSONEncoder().encode([channel]))
            case "/api/shapes/users":
                return (200, ["electric-handle": "people", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            case "/api/channel-read-states":
                return (200, [:], try JSONEncoder().encode([ChannelReadState(channel_id: channel.id, has_unread: false, last_read_at: nil)]))
            case "/api/channel-pins": throw URLError(.notConnectedToInternet)
            default: Issue.record("予期しないAPI呼び出し"); return (404, [:], Data())
            }
        }
        await workspace.refresh()
        #expect(workspace.hasLoaded && !workspace.directoryReady)
        if stalled {
            let changed = AsyncStream<Void>.makeStream()
            defer { changed.continuation.finish() }
            withObservationTracking { _ = workspace.pinsError } onChange: {
                changed.continuation.yield(())
                changed.continuation.finish()
            }
            harness.stub { _ in
                try await Task.sleep(for: .seconds(60))
                throw CancellationError()
            }
            let running = Task { await workspace.runPins(initialLoadTimeout: .zero) }
            defer { running.cancel() }
            for await _ in changed.stream { break }
            running.cancel()
            await running.value
        } else { await workspace.loadPins() }
        #expect(workspace.directoryReady && workspace.pinsError != nil)
        #expect(workspace.channels == [channel])
        harness.stub { _ in
            (200, [:], Data("{\"pins\":[{\"channel_id\":\"\(channel.id)\",\"position\":0}],\"version\":1}".utf8))
        }
        await workspace.loadPins()
        #expect(workspace.directoryReady && workspace.pinsError == nil)
        #expect(workspace.pinnedChannelIDs == [channel.id])
    }

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
        let added = try await harness.client.setChannelPinned(id: id, pinned: true, expectedVersion: 7)
        #expect(added.pins.first?.channel_id == id && added.version == 8)
        let removed = try await harness.client.setChannelPinned(id: id, pinned: false, expectedVersion: added.version)
        #expect(removed.pins.isEmpty && removed.version == 9)
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
                #expect(body["channel_id"] == .string(id))
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

    @Test("ピンは本人のShapeを1本購読し、空の更新でRESTを繰り返さず別端末の変更を反映する",
          .timeLimit(.minutes(1)), arguments: [false, true])
    func livePins(stringPosition: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let id = channel.id
        let shapes = Mutex(0), lists = Mutex(0)
        let checkpoints = AsyncStream<Int>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { checkpoints.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            if request.url?.path == "/api/channel-pins" {
                let count = lists.withLock { $0 += 1; return $0 }
                let pins = count == 1 ? "[]" : "[{\"channel_id\":\"\(id)\",\"position\":0}]"
                return (200, [:], Data("{\"pins\":\(pins),\"version\":\(count)}".utf8))
            }
            #expect(request.url?.path == "/api/shapes/user_pinned_channels")
            let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            #expect(!query.contains { $0.name == "channel_id" })
            let count = shapes.withLock { $0 += 1; return $0 }
            if count >= 3 {
                checkpoints.continuation.yield(count)
                if count == 3 { for await _ in release.stream { break } }
                else { try await Task.sleep(for: .seconds(60)) }
            }
            let position = stringPosition ? "\"0\"" : "0"
            let body = count == 3
                ? "[{\"key\":\"pin\",\"headers\":{\"operation\":\"insert\"},\"value\":{\"channel_id\":\"\(id)\",\"position\":\(position)}}]"
                : "[]"
            return (200, ["electric-handle": "pins", "electric-offset": "\(count)_0", "electric-up-to-date": "true"], Data(body.utf8))
        }
        let task = Task {
            defer { checkpoints.continuation.finish() }
            await workspace.runPins()
        }
        defer { task.cancel() }
        var iterator = checkpoints.stream.makeAsyncIterator()
        try #require(await iterator.next() == 3)
        #expect(lists.withLock { $0 } == 1)
        #expect(workspace.pinnedChannelIDs.isEmpty)
        release.continuation.finish()
        try #require(await iterator.next() == 4)
        #expect(lists.withLock { $0 } == 2)
        #expect(workspace.pinnedChannelIDs == [id])
        #expect(workspace.pinsError == nil)
        task.cancel()
        await task.value
        #expect(shapes.withLock { $0 } == 4)
        #expect(lists.withLock { $0 } == 2)
    }

    @Test("変更後のピン取得がタイムアウトしても古いキャッシュで同期完了にせず再試行する", .timeLimit(.minutes(1)))
    func livePinsRetriesAfterCachedFallback() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let user = Person(id: "you", user_name: "you", display_name: "あなた")
        let cache = try await SyncCache.open(inMemory: true)
        let sync = SyncSession(cache: cache, lease: try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: user))
        let workspace = WorkspaceStore(client: NativeChatClient(api: harness.api, sync: sync), user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let id = channel.id
        let recovered = Mutex(false), requests = Mutex(0)
        let failed = AsyncStream<Void>.makeStream()
        defer { failed.continuation.finish() }
        harness.stub { request in
            if request.url?.path == "/api/channel-pins" {
                let count = requests.withLock { $0 += 1; return $0 }
                if count == 1 { return (200, [:], Data(#"{"pins":[{"channel_id":"old","position":0}],"version":1}"#.utf8)) }
                if !recovered.withLock({ $0 }) {
                    if count == 3 { failed.continuation.yield(()) }
                    throw URLError(.timedOut)
                }
                return (200, [:], Data("{\"pins\":[{\"channel_id\":\"\(id)\",\"position\":0}],\"version\":2}".utf8))
            }
            #expect(request.url?.path == "/api/shapes/user_pinned_channels")
            let body = "[{\"key\":\"pin\",\"headers\":{\"operation\":\"insert\"},\"value\":{\"channel_id\":\"\(id)\",\"position\":\"0\"}}]"
            return (200, ["electric-handle": "pins", "electric-offset": "1_0", "electric-up-to-date": "true"], Data(body.utf8))
        }
        await workspace.loadPins()
        try #require(workspace.pinnedChannelIDs == ["old"])
        let task = Task { defer { failed.continuation.finish() }; await workspace.runPins() }
        defer { task.cancel() }
        var failures = failed.stream.makeAsyncIterator()
        try #require(await failures.next() != nil)
        #expect(workspace.pinnedChannelIDs == ["old"])
        recovered.withLock { $0 = true }
        while workspace.pinnedChannelIDs != [id] {
            let update = AsyncStream<Void>.makeStream()
            withObservationTracking { _ = workspace.pinnedChannelIDs } onChange: {
                update.continuation.yield(())
                update.continuation.finish()
            }
            for await _ in update.stream { break }
            try Task.checkCancellation()
        }
        #expect(workspace.pinsError == nil)
        #expect(try await sync.load("rest:api/channel-pins", as: ChannelPins.self)?.pins.map(\.channel_id) == [id])
        task.cancel(); await task.value
    }

    @Test("ピンの不正な位置を0として受け入れない", arguments: ["null", "true", "1.5", "\"1.5\"", "\"9223372036854775808\"", "\"unknown\""])
    func rejectsInvalidPinPosition(_ position: String) {
        #expect(throws: (any Error).self) {
            _ = try JSONDecoder().decode(ChannelPin.self, from: Data("{\"channel_id\":\"c\",\"position\":\(position)}".utf8))
        }
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
        harness.stub { _ in (200, [:], Data(#"{"data":{"pins":[],"version":2},"txid":"2"}"#.utf8)) }
        await workspace.setChannelPinned(channel, pinned: false)
        #expect(workspace.pinnedChannelIDs.isEmpty)
        #expect(workspace.pinUpdateError == nil && !workspace.updatingPin)
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
