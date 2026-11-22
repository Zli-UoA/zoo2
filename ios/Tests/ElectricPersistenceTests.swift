import Foundation
import Synchronization
import Testing
@testable import Zoo

private let syncUser = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "sakura", display_name: "さくら")
private let syncChannel = "11111111-1111-4111-8111-111111111111"

private func syncResponse(offset: String = "100_0", status: Int = 200) throws -> HTTPURLResponse {
    try #require(HTTPURLResponse(url: URL(string: "https://api.test.invalid/api/shapes/messages")!, statusCode: status,
                                httpVersion: nil, headerFields: ["electric-handle": "h", "electric-offset": offset]))
}

private func syncRow(_ number: Int, parent: String? = nil) -> [String: JSONValue] {
    ["id": .string(String(format: "22222222-2222-4222-8222-%012d", number)), "channel_id": .string(syncChannel),
     "user_id": .string(syncUser.id), "content": .string("投稿\(number)"), "created_at": .string("2026-11-19 10:00:00"),
     "parent_id": parent.map(JSONValue.string) ?? .null, "display_data": .object(["reactions": .array([]), "reply_count": .number(2)])]
}

private func subsetData(_ numbers: [Int], parent: String? = nil) throws -> Data {
    let rows: [JSONValue] = numbers.map { number in
        .object(["key": .string("k\(number)"), "headers": .object(["operation": .string("insert")]),
                 "value": .object(syncRow(number, parent: parent))])
    }
    return try JSONEncoder().encode(JSONValue.object([
        "data": .array(rows), "metadata": .object([
            "xmin": .string("90"), "xmax": .string("100"), "xip_list": .array([.string("95")]),
            "database_lsn": .string("1000"), "snapshot_mark": .number(1)
        ])
    ]))
}

@Suite("SwiftDataの同期キャッシュ", .timeLimit(.minutes(1)))
struct SyncCacheTests {
    @Test("データと同期位置を保存し、コンテナを開き直して復元する")
    func reopensDiskStore() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("test.store")
        let cache = try await SyncCache.open(url: url)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        var state = ShapeState()
        state.handle = "h"; state.offset = "123_4"; state.isLive = true
        state.rows = ["k": syncRow(1)]
        try await cache.store(JSONEncoder().encode(state), resource: "messages", channelID: syncChannel, lease: lease)
        let reopened = try await SyncCache.open(url: url)
        #expect(try await reopened.profile(connection: "connection") == syncUser)
        let newLease = try await reopened.activate(connection: "connection", user: syncUser)
        let bytes = try #require(await reopened.load("messages", lease: newLease))
        let restored = try JSONDecoder().decode(ShapeState.self, from: bytes)
        #expect(restored.offset == "123_4")
        #expect(restored.rows["k"]?["content"] == .string("投稿1"))
    }

    @Test("アカウント切替後に古い処理から保存できず、他人のデータを読めない")
    func separatesAccounts() async throws {
        let cache = try await SyncCache.open(inMemory: true)
        let old = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data("秘密のメッセージ".utf8), resource: "messages", lease: old)
        let other = try await cache.activate(connection: "connection", user: Person(id: "other", user_name: "other", display_name: "別のユーザー"))
        #expect(try await cache.load("messages", lease: other) == nil)
        await #expect(throws: CancellationError.self) { try await cache.store(Data(), resource: "messages", lease: old) }
        try await cache.revoke(other)
        #expect(try await cache.profile(connection: "connection") == nil)
        await #expect(throws: CancellationError.self) { try await cache.load("messages", lease: other) }
    }

    @Test("通知削除の保存が失敗しても再起動後に隠し、新しい通知を保存した後はオフラインで復元する")
    func revocationSurvivesFailedSaveAndRestart() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("test.store")
        let fails = Mutex(false)
        let cache = try await SyncCache.open(url: url, beforeSave: {
            if fails.withLock({ $0 }) { throw CocoaError(.fileWriteOutOfSpace) }
        })
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        let resources = ["shape:api/shapes/notifications", "snapshot:api/shapes/notifications"]
        for resource in resources { try await cache.store(Data("失効前の通知".utf8), resource: resource, lease: lease) }
        try await cache.store(Data("別の情報".utf8), resource: "other", lease: lease)
        fails.withLock { $0 = true }
        await #expect(throws: CocoaError.self) { try await cache.revokeChannel(syncChannel, lease: lease) }
        for resource in resources { #expect(try await cache.load(resource, lease: lease) == nil) }

        let reopened = try await SyncCache.open(url: url)
        let next = try await reopened.activate(connection: "connection", user: syncUser)
        for resource in resources { #expect(try await reopened.load(resource, lease: next) == nil) }
        #expect(try await reopened.load("other", lease: next) == Data("別の情報".utf8))
        for resource in resources { try await reopened.store(Data("再取得後の通知".utf8), resource: resource, lease: next) }

        let offline = try await SyncCache.open(url: url)
        let restored = try await offline.activate(connection: "connection", user: syncUser)
        for resource in resources { #expect(try await offline.load(resource, lease: restored) == Data("再取得後の通知".utf8)) }
    }

    @Test("権限を失った会話と通知だけを削除する")
    func revokesChannel() async throws {
        let cache = try await SyncCache.open(inMemory: true)
        let lease = try await cache.activate(connection: "connection", user: syncUser)
        try await cache.store(Data(), resource: "messages", channelID: syncChannel, lease: lease)
        try await cache.store(Data(), resource: "other", channelID: "other", lease: lease)
        try await cache.store(Data(), resource: "shape:api/shapes/notifications", lease: lease)
        try await cache.revokeChannel(syncChannel, lease: lease)
        #expect(try await cache.load("messages", lease: lease) == nil)
        #expect(try await cache.load("shape:api/shapes/notifications", lease: lease) == nil)
        #expect(try await cache.load("other", lease: lease) != nil)
    }
}

@Suite("Electricの履歴とSSE", .timeLimit(.minutes(1)))
struct ElectricProtocolTests {
    @Test("UTF-8と複数data行をLF・CRLFで読み、コメントを無視する", arguments: ["\n", "\r\n", "\r"])
    func parsesSSE(newline: String) throws {
        let input = [": keep-alive", "", "data: {", "data: \"内容\":\"さくら🌸\"}", "", "data: 未完了"]
            .joined(separator: newline)
        var parser = ElectricSSEParser()
        var values: [Data] = []
        for byte in input.utf8 { if let value = try parser.feed(byte) { values.append(value) } }
        #expect(values.count == 1)
        #expect(try JSONDecoder().decode([String: String].self, from: #require(values.first)) == ["内容": "さくら🌸"])
    }

    @Test("subsetに含まれる古い変更を除外し、実行中だった変更は反映する")
    func snapshotVisibility() throws {
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "80_0"
        _ = try state.addSnapshot(subsetData([1]), response: syncResponse())
        let old = Data(#"[{"key":"k1","headers":{"operation":"update","txids":[94]},"value":{"content":"古い本文"}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        try state.apply(old, response: syncResponse())
        #expect(state.shape.rows["k1"]?["content"] == .string("投稿1"))
        let concurrent = Data(#"[{"key":"k1","headers":{"operation":"update","txids":[95]},"value":{"content":"同時更新"}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        try state.apply(concurrent, response: syncResponse(offset: "101_0"))
        #expect(state.shape.rows["k1"]?["content"] == .string("同時更新"))
        let data = try JSONEncoder().encode(state)
        #expect(try JSONDecoder().decode(MessageSyncState.self, from: data).boundaries.first?.keys == ["k1"])
    }

    @Test("取得範囲外の古い投稿を保存せず、読んでいる範囲の新着を反映する")
    func limitsLiveRows() throws {
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "80_0"
        state.windows["root"] = MessageWindow(parentID: nil, limit: 50, hasMore: true,
                                               oldestDate: "2026-11-19 10:00:00", oldestID: syncRow(50)["id"]?.text)
        let events: [JSONValue] = [1, 100].map { number in
            .object(["key": .string("k\(number)"), "headers": .object(["operation": .string("update")]), "value": .object(syncRow(number))])
        }
        try state.apply(JSONEncoder().encode(events), response: syncResponse())
        #expect(state.shape.rows.keys.sorted() == ["k100"])
    }

    @Test("409で履歴とスナップショット境界を破棄する")
    func resetsWindow() throws {
        var state = MessageSyncState()
        state.shape.handle = "h"
        _ = try state.addSnapshot(subsetData([1]), response: syncResponse())
        try state.apply(Data(), response: syncResponse(status: 409))
        #expect(state.shape.rows.isEmpty)
        #expect(state.boundaries.isEmpty)
        #expect(state.shape.offset == "now")
    }

    @Test("所属変更をタグのOR・AND条件に適用し、ログを巻き戻さない")
    func appliesMovesWithPersistedTags() throws {
        var state = ShapeState()
        let rows = Data(#"[{"key":"private","value":{"id":"private"},"headers":{"operation":"insert","tags":["member/","/1"],"active_conditions":[true,false]}},{"key":"public","value":{"id":"public"},"headers":{"operation":"insert","tags":["member/","/1"],"active_conditions":[true,true]}},{"key":"and","value":{"id":"and"},"headers":{"operation":"insert","tags":["member/other"],"active_conditions":[true,true]}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(rows, response: syncResponse(), as: JSONValue.self)
        state = try JSONDecoder().decode(ShapeState.self, from: JSONEncoder().encode(state))
        let moveOut = Data(#"[{"headers":{"event":"move-out","patterns":[{"pos":0,"value":"member"}]}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(moveOut, response: syncResponse(offset: "101_0"), as: JSONValue.self)
        #expect(state.rows.keys.sorted() == ["public"])
        #expect(state.visibility["public"]?.conditions == [false, true])
        #expect(state.offset == "101_0")
        #expect(state.handle == "h")
        let moveIn = Data(#"[{"headers":{"event":"move-in","patterns":[{"pos":0,"value":"member"}]}},{"headers":{"event":"move-out","patterns":[{"pos":1,"value":"1"}]}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(moveIn, response: syncResponse(offset: "102_0"), as: JSONValue.self)
        #expect(state.rows.keys.sorted() == ["public"])
        #expect(state.visibility["public"]?.conditions == [true, false])
        _ = try state.apply(moveOut, response: syncResponse(offset: "103_0"), as: JSONValue.self)
        #expect(state.rows.isEmpty)
        #expect(state.visibility.isEmpty)
        #expect(state.offset == "103_0")
    }

    @Test("条件配列のないタグも追加と削除を反映する")
    func appliesSimpleTags() throws {
        var state = ShapeState()
        let rows = Data(#"[{"key":"k","value":{"id":"k"},"headers":{"operation":"insert","tags":["a/","b/"]}},{"key":"k","value":{"id":"k"},"headers":{"operation":"update","tags":["c/"],"removed_tags":["a/"]}},{"headers":{"event":"move-out","patterns":[{"pos":0,"value":"b"}]}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(rows, response: syncResponse(), as: JSONValue.self)
        #expect(state.visibility["k"]?.tags == ["c/"])
        #expect(state.rows["k"] != nil)
        let move = Data(#"[{"headers":{"event":"move-out","patterns":[{"pos":0,"value":"c"}]}},{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(move, response: syncResponse(offset: "101_0"), as: JSONValue.self)
        #expect(state.rows.isEmpty)
    }
}

@MainActor
@Suite("永続同期の通信経路", .timeLimit(.minutes(1)))
struct PersistentSyncIntegrationTests {
    private func session(_ harness: ChatHarness) async throws -> SyncSession {
        let cache = try await SyncCache.open(inMemory: true)
        let lease = try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: syncUser)
        return SyncSession(cache: cache, lease: lease)
    }

    @Test("再取得で保存したoffsetを使い、オフラインでは取得済みの行を返す")
    func resumesSnapshot() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let requests = Mutex<[String]>([])
        harness.stub { request in
            let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            requests.withLock { $0.append(query.first { $0.name == "offset" }?.value ?? "") }
            let count = requests.withLock { $0.count }
            if count == 3 { throw URLError(.notConnectedToInternet) }
            let events: [JSONValue] = count == 1
                ? [.object(["key": .string("k1"), "headers": .object(["operation": .string("insert")]), "value": .object(syncRow(1))])]
                : []
            return (200, ["electric-handle": "h", "electric-offset": "\(count)_0", "electric-up-to-date": "true"], try JSONEncoder().encode(events))
        }
        let stream = ShapeStream(api: harness.api, sync: sync)
        for _ in 0..<3 {
            #expect(try await stream.snapshot("api/shapes/messages", query: ["channel_id": syncChannel], as: Message.self).count == 1)
        }
        #expect(requests.withLock { $0 } == ["-1", "1_0", "2_0"])
    }

    @Test("差分取得の途中で切断しても最後に確定したスナップショットを残す")
    func preservesSnapshotDuringFailedCatchup() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var state = ShapeState()
        state.handle = "h"; state.offset = "100_0"; state.isLive = true
        state.rows = ["k1": syncRow(1)]
        try await sync.store(state, key: "snapshot:api/shapes/users")
        let calls = Mutex(0)
        harness.stub { _ in
            let count = calls.withLock { $0 += 1; return $0 }
            if count == 1 { return (200, ["electric-handle": "h", "electric-offset": "101_0"], Data("[]".utf8)) }
            throw URLError(.notConnectedToInternet)
        }
        for _ in 0..<2 {
            let messages = try await ShapeStream(api: harness.api, sync: sync).snapshot("api/shapes/users", as: Message.self)
            #expect(messages.count == 1)
        }
        let saved = try #require(await sync.load("snapshot:api/shapes/users", as: ShapeState.self))
        #expect(saved.isLive)
        #expect(saved.offset == "100_0")
    }

    @Test("オフラインで会話を繰り返し開いても確定済みの履歴が残る")
    func preservesOfflineMessages() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "100_0"; state.shape.isLive = true
        _ = try state.addSnapshot(subsetData([1]), response: syncResponse())
        state.windows["root"] = MessageWindow(parentID: nil, limit: 50, hasMore: false)
        let key = "messages-v1:" + syncChannel
        try await sync.store(state, key: key, channelID: syncChannel)
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        for _ in 0..<2 {
            var snapshots: [MessageFeedSnapshot] = []
            await #expect(throws: URLError.self) {
                try await NativeChatClient(api: harness.api, sync: sync).observeMessageFeed(channelID: syncChannel,
                                                                                         request: MessageFeedRequest()) {
                    snapshots.append($0)
                }
            }
            #expect(snapshots.count == 1)
            #expect(snapshots.first?.fromCache == true)
            #expect(snapshots.first?.messages?.first?.content == "投稿1")
            #expect(try await sync.load(key, as: MessageSyncState.self)?.shape.isLive == true)
        }
    }

    @Test("再招待をサーバーで確認したら保存を再開し、失効前の応答は拒否する")
    func restoresChannelAccess() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let oldVersion = sync.channelVersion(syncChannel)
        try await sync.store("失効前", key: "messages", channelID: syncChannel, channelVersion: oldVersion)
        harness.stub { _ in (404, [:], Data()) }
        await #expect(throws: ZooError.self) {
            _ = try await NativeChatClient(api: harness.api, sync: sync).channel(id: syncChannel)
        }
        #expect(try await sync.load("messages", as: String.self) == nil)
        await #expect(throws: CancellationError.self) {
            try await sync.store("遅い応答", key: "messages", channelID: syncChannel, channelVersion: oldVersion)
        }
        harness.stub { request in
            #expect(request.url?.path == "/api/channels/" + syncChannel)
            return (200, [:], try JSONEncoder().encode(Channel(id: syncChannel, channel_name: "再招待", is_private: true, is_dm: false)))
        }
        _ = try await NativeChatClient(api: harness.api, sync: sync).channel(id: syncChannel)
        #expect(!sync.isRevoked(syncChannel))
        let newVersion = sync.channelVersion(syncChannel)
        try await sync.store("復旧後", key: "messages", channelID: syncChannel, channelVersion: newVersion)
        await #expect(throws: CancellationError.self) {
            try await sync.store("遅い応答", key: "messages", channelID: syncChannel, channelVersion: oldVersion)
        }
        await #expect(throws: CancellationError.self) {
            try await sync.cache.store(Data(), resource: "messages", channelID: syncChannel, channelVersion: oldVersion, lease: sync.lease)
        }
        #expect(try await sync.load("messages", as: String.self) == "復旧後")
    }

    @Test("会話取得中の失効やログアウト後に古い成功応答を返さない", arguments: [false, true])
    func rejectsLateChannel(logout: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        harness.stub { _ in
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, [:], try JSONEncoder().encode(Channel(id: syncChannel, channel_name: "失効前", is_private: true, is_dm: false)))
        }
        let pending = Task { try await NativeChatClient(api: harness.api, sync: sync).channel(id: syncChannel) }
        for await _ in started.stream { break }
        if logout { try await sync.close().value }
        else { try await sync.revokeChannel(syncChannel) }
        release.continuation.yield(())
        await #expect(throws: CancellationError.self) { _ = try await pending.value }
    }

    @Test("権限喪失前の通知応答で本文を再保存・再表示しない", arguments: [false, true])
    func rejectsLateNotifications(observe: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        let notices = Data("""
        [{"key":"n","headers":{"operation":"insert"},"value":{"id":"n","user_id":"\(syncUser.id)","actor_id":"a","channel_id":"\(syncChannel)","message_id":null,"notification_type":"mention","created_at":"2026-11-19 10:00:00","is_read":false,"display_data":{"message":{"content":"削除する本文"}}}},{"headers":{"control":"up-to-date"}}]
        """.utf8)
        harness.stub { request in
            if request.url?.path == "/api/activitypub/notifications" {
                return (200, [:], Data(#"{"items":[],"unread_count":0,"next_cursor":null}"#.utf8))
            }
            return (200, ["electric-handle": "h", "electric-offset": "100_0"], notices)
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        let inbox = InboxStore(client: client, userID: syncUser.id)
        sync.onChannelRevoked = { [weak inbox] id in inbox?.revokeChannel(id) }
        await inbox.refresh()
        #expect(inbox.entries.count == 1)
        let oldVersion = sync.notificationVersion(for: "api/shapes/notifications")
        harness.stub { _ in
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, ["electric-handle": "h", "electric-offset": "101_0"], notices)
        }
        var published = false
        let pending = Task {
            if observe { try await client.observeInbox { _ in published = true } }
            else { _ = try await client.channelNoticeSnapshot(); published = true }
        }
        for await _ in started.stream { break }
        try await sync.revokeChannel(syncChannel)
        #expect(inbox.entries.isEmpty)
        release.continuation.yield(())
        await #expect(throws: SyncInvalidation.self) { try await pending.value }
        #expect(!published)
        for key in ["shape:api/shapes/notifications", "snapshot:api/shapes/notifications"] {
            #expect(try await sync.load(key, as: ShapeState.self) == nil)
            await #expect(throws: SyncInvalidation.self) {
                try await sync.cache.store(Data(), resource: key, notificationVersion: oldVersion, lease: sync.lease)
            }
        }
    }

    @Test("SSEはup-to-dateまでまとめ、未完了の末尾を公開しない")
    func sseBatch() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.value(forHTTPHeaderField: "Accept") == "text/event-stream")
            let data = """
            data: {"key":"k1","headers":{"operation":"insert"},"value":{"id":"1"}}

            data: {"headers":{"control":"up-to-date","global_last_seen_lsn":"123"}}

            data: {"key":"k2","headers":{"operation":"insert"},"value":{"id":"2"}}


            """
            return (200, ["Content-Type": "text/event-stream", "electric-handle": "h"], Data(data.utf8))
        }
        var batches = 0
        do {
            _ = try await ElectricTransport(api: harness.api).read("api/shapes/messages", query: ["live": "true"], sse: true) { data, response in
            batches += 1
            #expect(response.value(forHTTPHeaderField: "electric-offset") == "123_0")
            let events = try JSONDecoder().decode([ShapeEvent].self, from: data)
            #expect(events.count == 2)
        }
            Issue.record("同期途中の EOF が受け入れられました。")
        } catch ZooError.invalidResponse { }
        #expect(batches == 1)
    }

    private func feed(_ client: NativeChatClient, request: MessageFeedRequest) async throws -> MessageFeedSnapshot {
        let updates = AsyncStream<MessageFeedSnapshot>.makeStream()
        let task = Task {
            defer { updates.continuation.finish() }
            try await client.observeMessageFeed(channelID: syncChannel, request: request) { updates.continuation.yield($0) }
        }
        for await value in updates.stream where !value.fromCache && value.messages != nil {
            task.cancel()
            _ = await task.result
            return value
        }
        try await task.value
        throw ZooError.invalidResponse
    }

    @Test("最初の50件、次の50件、保存位置からの再開で全履歴を取り直さない")
    func pagesAndResumption() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let requests = Mutex<[[String: String]]>([])
        harness.stub { request in
            let query = Dictionary((URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []).map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { _, new in new })
            requests.withLock { $0.append(query) }
            if let predicate = query["subset__where"] {
                #expect(query["subset__limit"] == "50")
                if predicate.contains("created_at <") {
                    let parameters = try JSONDecoder().decode([String: String].self, from: Data((query["subset__params"] ?? "").utf8))
                    #expect(parameters.keys.sorted() == ["1", "2"])
                    #expect(predicate.contains("created_at < $1"))
                    #expect(predicate.contains("id < $2"))
                }
                let numbers = predicate.contains("created_at <") ? Array((1...50).reversed()) : Array((51...100).reversed())
                return (200, ["electric-handle": "h", "electric-offset": "100_0"], try subsetData(numbers))
            }
            return (200, ["electric-handle": "h", "electric-offset": "100_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        #expect(try await feed(client, request: MessageFeedRequest()).messages?.count == 50)
        #expect(try await feed(client, request: MessageFeedRequest(limit: 100)).messages?.count == 100)
        #expect(try await feed(client, request: MessageFeedRequest(limit: 100)).messages?.count == 100)
        let values = requests.withLock { $0 }
        #expect(values.filter { $0["subset__where"] != nil }.count == 2)
        #expect(values.filter { $0["offset"] == "now" }.count == 1)
        #expect(!values.contains { $0["offset"] == "-1" })
    }

    @Test("保存済みの100件に続く50件を要求し、オフライン・重複・別スレッドの追加取得を止める")
    func storePaginationResumesLoadedWindow() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        var state = MessageSyncState()
        state.shape.handle = "h"; state.shape.offset = "100_0"; state.shape.isLive = true
        _ = try state.addSnapshot(subsetData(Array(51...150)), response: syncResponse())
        state.windows["root"] = MessageWindow(parentID: nil, limit: 100, hasMore: true,
                                               oldestDate: "2026-11-19 10:00:00", oldestID: syncRow(51)["id"]?.text)
        try await sync.store(state, key: "messages-v1:" + syncChannel, channelID: syncChannel)
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        let ready = AsyncStream<Void>.makeStream()
        let firstRequest = Mutex(true)
        let subsets = Mutex(0)
        harness.stub { request in
            let query = Dictionary((URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []).map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { _, new in new })
            if firstRequest.withLock({ value in defer { value = false }; return value }) {
                started.continuation.yield(())
                for await _ in release.stream { break }
            }
            if query["subset__where"] != nil {
                subsets.withLock { $0 += 1 }
                return (200, ["electric-handle": "h", "electric-offset": "100_0"], try subsetData(Array(1...50)))
            }
            if query["live"] == "true" {
                ready.continuation.yield(())
                try await Task.sleep(for: .seconds(60))
            }
            return (200, ["electric-handle": "h", "electric-offset": "100_0", "electric-up-to-date": "true"], Data("[]".utf8))
        }
        let store = ConversationStore(channel: Channel(id: syncChannel, channel_name: "履歴", is_private: false, is_dm: false),
                                      client: NativeChatClient(api: harness.api, sync: sync), userID: syncUser.id)
        #expect(store.postingRestrictionMessage == nil)
        var run = Task { await store.run() }
        defer { run.cancel() }
        for await _ in started.stream { break }
        #expect(store.showingCachedMessages)
        store.drafts["root"] = "同期中に入力した下書き"
        #expect(!store.canSend(parentID: nil))
        #expect(store.postingRestrictionMessage == nil)
        var channel = store.channel
        channel.archive_at = "2026-11-20T00:00:00Z"
        store.update(channel)
        #expect(store.postingRestrictionMessage == "アーカイブ済みのチャンネルです")
        channel.archive_at = nil
        store.update(channel)
        #expect(store.postingRestrictionMessage == nil)
        let initialRequest = store.syncRequest
        store.loadOlder(parentID: nil)
        #expect(store.syncRequest == initialRequest)
        release.continuation.yield(())
        var iterator = ready.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(!store.showingCachedMessages && store.messages.count == 100)
        #expect(store.drafts["root"] == "同期中に入力した下書き")
        #expect(store.canSend(parentID: nil))
        #expect(store.postingRestrictionMessage == nil)
        #expect(store.historyRevision == 0)
        store.loadOlder(parentID: "別スレッド")
        #expect(store.syncRequest == initialRequest)
        store.loadOlder(parentID: nil)
        #expect(store.loadingOlder && store.syncRequest != initialRequest)
        let nextRequest = store.syncRequest
        store.loadOlder(parentID: nil)
        #expect(store.syncRequest == nextRequest)
        run.cancel()
        await run.value
        run = Task { await store.run() }
        _ = await iterator.next()
        #expect(store.messages.count == 150)
        #expect(store.historyRevision == 1)
        #expect(!store.loadingOlder)
        #expect(subsets.withLock { $0 } == 1)
    }

    @Test("保存したログイン状態があれば通信なしでワークスペースを復元する")
    func offlineLaunch() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let sync = try await session(harness)
        let preferenceKey = "zoo.push.enabled." + PushReceipt.digest(harness.api.settings.storageKey + "|" + syncUser.id)
        UserDefaults.standard.set(true, forKey: preferenceKey)
        defer { UserDefaults.standard.removeObject(forKey: preferenceKey) }
        try await sync.store([Channel(id: syncChannel, channel_name: "保存済み", is_private: false, is_dm: false)], key: "workspace:channels")
        try await sync.store([syncUser], key: "workspace:people")
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        let app = AppSession(settings: harness.api.settings, transport: harness.api.transport,
                             notificationStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys(), syncCache: sync.cache)
        await app.resume()
        #expect(app.userID == syncUser.id)
        let workspace = try #require(app.workspace)
        #expect(try await workspace.client.cachedWorkspace()?.channels.first?.channel_name == "保存済み")
        #expect(app.notifications.receive(.test(userID: syncUser.id, id: UUID())))
        let verified = Mutex(false)
        harness.stub { request in
            #expect(request.url?.path == "/api/users/me")
            verified.withLock { $0 = true }
            return (200, [:], try JSONEncoder().encode(syncUser))
        }
        await app.resume()
        #expect(verified.withLock { $0 })
        #expect(app.workspace === workspace)
        app.logout()
        #expect(app.workspace == nil)
    }
}
