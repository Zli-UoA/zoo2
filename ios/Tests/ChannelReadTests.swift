import Foundation
import Synchronization
import Testing
@testable import Zoo

private let readChannel = Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "一般", is_private: false, is_dm: false)
private let readOtherChannel = Channel(id: "55555555-5555-4555-8555-555555555555", channel_name: "開発", is_private: false, is_dm: false)
private let readMessageID = "22222222-2222-4222-8222-222222222222"

private func readSnapshot(channelID: String = readChannel.id, parentID: String? = nil) throws -> Data {
    try JSONEncoder().encode(JSONValue.object(["data": .array([
        .object(["headers": .object(["operation": .string("insert")]), "key": .string(readMessageID),
                 "value": .object(["id": .string(readMessageID), "channel_id": .string(channelID),
                                   "parent_id": parentID.map(JSONValue.string) ?? .null, "deleted_at": .null])])
    ])]))
}

private func readResponse(unread: Bool = false) throws -> Data {
    let state = ChannelReadState(channel_id: readChannel.id, has_unread: unread, last_read_at: "2026-11-21T10:00:00Z")
    let value = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(state))
    return try JSONEncoder().encode(["data": value])
}

private func readWorkspaceResponse(_ request: URLRequest) throws -> (Int, [String: String], Data) {
    switch request.url?.path {
    case "/api/channels": return (200, [:], try JSONEncoder().encode([readChannel, readOtherChannel]))
    case "/api/channel-read-states":
        return (200, [:], try JSONEncoder().encode([readChannel, readOtherChannel].map {
            ChannelReadState(channel_id: $0.id, has_unread: true, last_read_at: nil)
        }))
    case "/api/shapes/users":
        return (200, ["electric-handle": "users", "electric-offset": "0_1"], Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
    case "/api/shapes/messages": return (200, [:], try readSnapshot())
    default: Issue.record("想定外の通信です：\(request.url?.path ?? "")"); return (500, [:], Data())
    }
}

@MainActor
@Suite("チャンネル一覧からの一括既読", .timeLimit(.minutes(1)))
struct ChannelReadTests {
    private func workspace(_ harness: ChatHarness) -> WorkspaceStore {
        WorkspaceStore(client: harness.client, user: Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
    }

    @Test("最新の親投稿を1件取得し、そのIDを既読APIへ送る")
    func latestMessageRequest() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let requests = Mutex<[String]>([])
        harness.stub { request in
            requests.withLock { $0.append(request.httpMethod ?? "") }
            if request.httpMethod == "GET" {
                #expect(request.url?.path == "/api/shapes/messages")
                let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
                let values = Dictionary(uniqueKeysWithValues: query.map { ($0.name, $0.value ?? "") })
                #expect(values["channel_id"] == readChannel.id)
                #expect(values["log"] == "changes_only")
                #expect(values["subset__limit"] == "1")
                #expect(values["subset__where"] == "parent_id IS NULL AND deleted_at IS NULL")
                #expect(values["subset__order_by"] == "created_at DESC, id DESC")
                return (200, [:], try readSnapshot())
            }
            #expect(request.url?.path == "/api/channels/\(readChannel.id)/read-state")
            let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
            #expect(body == ["message_id": .string(readMessageID)])
            return (200, [:], try readResponse())
        }
        let state = try await harness.client.markChannelRead(channelID: readChannel.id)
        #expect(!state.has_unread)
        #expect(requests.withLock { $0 } == ["GET", "POST"])
    }

    @Test("空のチャンネルでは架空の既読位置を保存せず現在の状態を取得する")
    func emptyChannel() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.httpMethod == "GET")
            if request.url?.path == "/api/shapes/messages" { return (200, [:], Data(#"{"data":[]}"#.utf8)) }
            #expect(request.url?.path == "/api/channels/\(readChannel.id)/read-state")
            return (200, [:], try JSONEncoder().encode(ChannelReadState(channel_id: readChannel.id, has_unread: false, last_read_at: nil)))
        }
        #expect(try await !harness.client.markChannelRead(channelID: readChannel.id).has_unread)
    }

    @Test("別チャンネルや返信の応答では既読を送信しない", arguments: [false, true])
    func invalidLatestMessage(reply: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.httpMethod == "GET")
            return (200, [:], try readSnapshot(channelID: reply ? readChannel.id : readOtherChannel.id, parentID: reply ? readMessageID : nil))
        }
        await #expect(throws: ZooError.self) { _ = try await harness.client.markChannelRead(channelID: readChannel.id) }
    }

    @Test("別チャンネルの保存完了で単件の既読状態を取り直さない")
    func independentReadWrites() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let rereads = Mutex(0)
        harness.stub { request in
            let path = request.url?.path ?? ""
            if path == "/api/shapes/messages" {
                let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
                return (200, [:], try readSnapshot(channelID: try #require(query.first { $0.name == "channel_id" }?.value)))
            }
            if path.hasSuffix("/read-state") {
                if request.httpMethod == "POST", path.contains(readChannel.id) {
                    started.continuation.yield(())
                    for await _ in release.stream { break }
                } else if request.httpMethod == "GET" { rereads.withLock { $0 += 1 } }
                let id = path.contains(readChannel.id) ? readChannel.id : readOtherChannel.id
                return (200, [:], try JSONEncoder().encode(["data": ChannelReadState(channel_id: id, has_unread: false, last_read_at: nil)]))
            }
            return try readWorkspaceResponse(request)
        }
        let store = workspace(harness)
        defer { store.shutdown() }
        await store.refresh()
        let first = Task {
            defer { started.continuation.finish() }
            await store.markChannelRead(readChannel)
        }
        defer { first.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        await store.markChannelRead(readOtherChannel)
        release.continuation.finish()
        await first.value
        #expect(store.unread.isEmpty && store.readUpdateError == nil)
        #expect(rereads.withLock { $0 } == 0)
    }

    @Test("対象だけ既読にし、保存中の新着がある場合は未読を残す", arguments: [false, true])
    func appliesServerState(hasNewMessage: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            if request.httpMethod == "POST" { return (200, [:], try readResponse(unread: hasNewMessage)) }
            return try readWorkspaceResponse(request)
        }
        let store = workspace(harness)
        defer { store.shutdown() }
        await store.refresh()
        await store.markChannelRead(readChannel)
        #expect(store.unread.contains(readChannel.id) == hasNewMessage)
        #expect(store.unread.contains(readOtherChannel.id))
        #expect(store.markingReadChannelIDs.isEmpty)
        #expect(store.readUpdateError == nil)
        #expect(store.channelPath.isEmpty)
    }

    @Test("保存失敗では未読を残し、再試行の成功でエラーを消す")
    func retriesFailure() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let writes = Mutex(0)
        harness.stub { request in
            if request.httpMethod == "POST" {
                let count = writes.withLock { $0 += 1; return $0 }
                if count == 1 { return (503, [:], Data()) }
                return (200, [:], try readResponse())
            }
            return try readWorkspaceResponse(request)
        }
        let store = workspace(harness)
        defer { store.shutdown() }
        await store.refresh()
        await store.markChannelRead(readChannel)
        #expect(store.unread.contains(readChannel.id))
        #expect(store.readUpdateError != nil)
        #expect(store.markingReadChannelIDs.isEmpty)
        await store.markChannelRead(readChannel)
        #expect(!store.unread.contains(readChannel.id))
        #expect(store.readUpdateError == nil)
    }

    @Test("重複操作を送らずログアウト後の応答を反映しない")
    func duplicateAndShutdown() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let writes = Mutex(0)
        harness.stub { request in
            if request.httpMethod == "POST" {
                let attempt = writes.withLock { $0 += 1; return $0 }
                guard attempt == 1 else {
                    Issue.record("重複した既読要求が送信されました。")
                    throw ZooError.invalidResponse
                }
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                return (200, [:], try readResponse(unread: true))
            }
            return try readWorkspaceResponse(request)
        }
        let store = workspace(harness)
        await store.refresh()
        let pending = Task {
            defer { started.continuation.finish() }
            await store.markChannelRead(readChannel)
        }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        #expect(store.markingReadChannelIDs == [readChannel.id])
        await store.markChannelRead(readChannel)
        #expect(writes.withLock { $0 } == 1)
        store.shutdown()
        release.continuation.yield(())
        await pending.value
        #expect(store.unread.isEmpty)
        #expect(store.markingReadChannelIDs.isEmpty)
        #expect(store.readUpdateError == nil)
    }

    @Test("一覧取得中の既読保存後は新着を含む最新状態を取り直す", arguments: [false, true], [false, true])
    func overlappingRefresh(hasNewMessage: Bool, snapshotUnread: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let reads = Mutex(0)
        harness.stub { request in
            if request.httpMethod == "POST" { return (200, [:], try readResponse()) }
            if request.url?.path == "/api/channel-read-states" {
                let count = reads.withLock { $0 += 1; return $0 }
                if count == 2 {
                    started.continuation.yield(())
                    var iterator = release.stream.makeAsyncIterator()
                    _ = await iterator.next()
                }
                let unread = count == 1 || (count == 2 ? snapshotUnread : hasNewMessage)
                return (200, [:], try JSONEncoder().encode([readChannel, readOtherChannel].map {
                    ChannelReadState(channel_id: $0.id, has_unread: $0.id == readOtherChannel.id || unread, last_read_at: nil)
                }))
            }
            return try readWorkspaceResponse(request)
        }
        let store = workspace(harness)
        defer { store.shutdown() }
        await store.refresh()
        let refreshing = Task {
            defer { started.continuation.finish() }
            await store.refresh()
        }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        await store.markChannelRead(readChannel)
        release.continuation.yield(())
        await refreshing.value
        #expect(store.unread.contains(readChannel.id) == hasNewMessage)
        #expect(store.unread.contains(readOtherChannel.id))
        #expect(reads.withLock { $0 } == 3)
        #expect(store.error == nil)
    }

    @Test("遅れて届く既読応答で一覧が反映した新着を消さない", arguments: [false, true])
    func delayedMarkResponse(reconciliationFails: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let reconciliations = Mutex(0)
        harness.stub { request in
            if request.httpMethod == "POST" {
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                return (200, [:], try readResponse())
            }
            if request.url?.path == "/api/channels/\(readChannel.id)/read-state" {
                reconciliations.withLock { $0 += 1 }
                if reconciliationFails { throw URLError(.notConnectedToInternet) }
                return (200, [:], try JSONEncoder().encode(ChannelReadState(channel_id: readChannel.id, has_unread: true, last_read_at: nil)))
            }
            return try readWorkspaceResponse(request)
        }
        let store = workspace(harness)
        defer { store.shutdown() }
        await store.refresh()
        let pending = Task {
            defer { started.continuation.finish() }
            await store.markChannelRead(readChannel)
        }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        await store.refresh()
        release.continuation.yield(())
        await pending.value
        #expect(store.unread.contains(readChannel.id))
        #expect(store.unread.contains(readOtherChannel.id))
        #expect(reconciliations.withLock { $0 } == 1)
        #expect((store.readUpdateError != nil) == reconciliationFails)
        #expect(store.markingReadChannelIDs.isEmpty)
    }

    @Test("競合解消の取得ではオフラインの古い既読キャッシュを採用しない")
    func reconciliationRequiresFreshState() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let cache = try await SyncCache.open(inMemory: true)
        let user = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")
        let lease = try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: user)
        let sync = SyncSession(cache: cache, lease: lease)
        let state = ChannelReadState(channel_id: readChannel.id, has_unread: false, last_read_at: nil)
        let client = NativeChatClient(api: harness.api, sync: sync)
        harness.stub { request in
            if request.url?.path == "/api/channel-read-states" {
                return (200, [:], try JSONEncoder().encode([state]))
            }
            #expect(request.url?.path == "/api/channels/\(readChannel.id)/read-state")
            return (200, [:], try JSONEncoder().encode(state))
        }
        _ = try await client.readState(channelID: readChannel.id)
        _ = try await client.readStates()
        harness.stub { _ in throw URLError(.notConnectedToInternet) }
        #expect(try await !client.readState(channelID: readChannel.id).has_unread)
        #expect(try await client.readStates().count == 1)
        await #expect(throws: (any Error).self) { _ = try await client.readState(channelID: readChannel.id, allowCached: false) }
        await #expect(throws: (any Error).self) { _ = try await client.readStates(allowCached: false) }
        try await sync.close().value
    }

    @Test("再参加したチャンネルを再認可してから既読にする")
    func reauthorizesRejoinedChannel() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let cache = try await SyncCache.open(inMemory: true)
        let user = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")
        let lease = try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: user)
        let sync = SyncSession(cache: cache, lease: lease)
        try await sync.revokeChannel(readChannel.id)
        let requests = Mutex<[String]>([])
        harness.stub { request in
            requests.withLock { $0.append(request.url?.path ?? "") }
            if request.url?.path == "/api/channels/\(readChannel.id)" {
                return (200, [:], try JSONEncoder().encode(readChannel))
            }
            if request.httpMethod == "POST" { return (200, [:], try readResponse()) }
            return try readWorkspaceResponse(request)
        }
        let state = try await NativeChatClient(api: harness.api, sync: sync).markChannelRead(channelID: readChannel.id)
        #expect(!state.has_unread)
        #expect(!sync.isRevoked(readChannel.id))
        #expect(requests.withLock { $0 } == ["/api/channels/\(readChannel.id)", "/api/shapes/messages", "/api/channels/\(readChannel.id)/read-state"])
        try await sync.close().value
    }
}
