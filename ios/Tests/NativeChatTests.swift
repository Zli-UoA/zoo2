import Foundation
import Observation
import Synchronization
import Testing
import XCTest
@testable import Zoo

@Suite("Electric の差分同期")
struct ShapeTests {
    private func response(offset: String = "0_1", status: Int = 200, handle: String = "shape-1") throws -> HTTPURLResponse {
        try #require(HTTPURLResponse(url: URL(string: "https://api.test.invalid/api/shapes/messages")!, statusCode: status,
                                    httpVersion: "HTTP/1.1", headerFields: ["electric-handle": handle, "electric-offset": offset]))
    }

    @Test("初期取得の完了を待ち、部分更新と削除を反映する")
    func snapshotAndPartialUpdates() throws {
        var state = ShapeState()
        let initial = Data(#"[{"key":"m1","headers":{"operation":"insert"},"value":{"id":"m1","channel_id":"c","user_id":"u","content":"最初","created_at":"2026-11-18 09:00:00"}}]"#.utf8)
        #expect(try state.apply(initial, response: response(), as: Message.self) == nil)
        #expect(state.query["live"] == nil)
        let ready = Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8)
        #expect(try state.apply(ready, response: response(offset: "0_2"), as: Message.self)?.first?.content == "最初")
        #expect(state.query["live"] == "true")
        let update = Data(#"[{"key":"m1","headers":{"operation":"update"},"value":{"content":"更新済み","updated_at":"2026-11-18 09:01:00"}}]"#.utf8)
        let changed = try #require(state.apply(update, response: response(offset: "0_3"), as: Message.self)?.first)
        #expect(changed.user_id == "u")
        #expect(changed.content == "更新済み")
        let deleted = Data(#"[{"key":"m1","headers":{"operation":"delete"},"value":{"id":"m1"}}]"#.utf8)
        #expect(try state.apply(deleted, response: response(offset: "0_4"), as: Message.self)?.isEmpty == true)
    }

    @Test("表示情報の一項目が不正でも他の項目を復号する", arguments: [false, true])
    func decodesDisplayFieldsIndependently(string: Bool) throws {
        let json = #"{"emojis":{"flower":{"url":"https://example.invalid/flower.png"}},"reactions":[],"reply_count":"不正"}"#
        let bytes = string ? try JSONEncoder().encode(json) : Data(json.utf8)
        let data = try JSONDecoder().decode(MessageDisplayData.self, from: bytes)
        #expect(data.emojis["flower"]?.url == "https://example.invalid/flower.png")
        #expect(data.reactions == [] && data.reply_count == nil)
    }

    @Test("shape 交換時に行とカーソルを破棄する", arguments: [200, 409])
    func reset(status: Int) throws {
        var state = ShapeState()
        _ = try state.apply(Data(#"[{"key":"m1","headers":{"operation":"insert"},"value":{"id":"m1","channel_id":"c","user_id":"u","content":"本文","created_at":"2026-11-18 09:00:00"}}]"#.utf8), response: response(), as: Message.self)
        #expect(state.rows["m1"] != nil)
        _ = try state.apply(Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8), response: response(), as: Message.self)
        let bytes = Data(#"[{"headers":{"control":"must-refetch"}}]"#.utf8)
        #expect(try state.apply(bytes, response: response(status: status, handle: "shape-2"), as: Message.self) == nil)
        #expect(state.rows.isEmpty)
        #expect(state.query["offset"] == "-1")
        #expect(state.query["handle"] == "shape-2")
        #expect(state.query["cache-buster"] != nil)
        #expect(state.query["live"] == nil)
    }

    @Test("不正な行を受け取った位置までカーソルを進めない")
    func invalidRowDoesNotCommitCursor() throws {
        var state = ShapeState()
        let ready = Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8)
        _ = try state.apply(ready, response: response(), as: Message.self)
        let malformed = Data(#"[{"key":"bad","headers":{"operation":"insert"},"value":{"id":"bad"}}]"#.utf8)
        #expect(throws: DecodingError.self) { try state.apply(malformed, response: response(offset: "0_9"), as: Message.self) }
        #expect(state.offset == "0_1")
        #expect(state.rows.isEmpty)
    }

    @Test("HTTP 204でも同期位置を維持する")
    func emptyLiveResponse() throws {
        var state = ShapeState()
        _ = try state.apply(Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8), response: response(offset: "9_4", handle: "known-shape"), as: Message.self)
        let before = state.query
        let empty = try #require(HTTPURLResponse(url: URL(string: "https://api.test.invalid/api/shapes/messages")!, statusCode: 204,
                                                 httpVersion: "HTTP/1.1", headerFields: [:]))
        #expect(try state.apply(Data(), response: empty, as: Message.self)?.isEmpty == true)
        #expect(state.isLive)
        #expect(state.query["offset"] == before["offset"])
        #expect(state.query["handle"] == before["handle"])
    }

    @Test("バックエンドと Electric の日時を UTC として読む", arguments: [
        "2026-11-18 09:00:00", "2026-11-18T09:00:00Z", "2026-11-18 09:00:00.123456",
        "2026-11-18T09:00:00.123456789Z", "2026-11-18T18:00:00+09:00"
    ])
    func dates(raw: String) throws {
        let date = try #require(ZooDate.parse(raw))
        #expect(abs(date.timeIntervalSince1970 - 1_794_992_400) < 1)
    }
}

@MainActor
final class ChatHarness {
    let id = UUID().uuidString
    let api: APIClient
    var client: NativeChatClient { NativeChatClient(api: api) }

    init() throws {
        let settings = ConnectionSettings(server: "https://api.test.invalid", issuer: "https://id.test.invalid", clientID: UUID().uuidString)
        try TokenVault.save(TokenSet(access: "test-access", refresh: nil, expires: Date().addingTimeInterval(3600)), account: settings.storageKey)
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StubProtocol.self]
        config.httpAdditionalHeaders = ["X-Zoo-Test-ID": id]
        let transport = HTTPTransport(session: URLSession(configuration: config))
        api = APIClient(settings: settings, auth: Authentication(settings: settings, transport: transport), transport: transport)
    }

    func stub(_ handler: @escaping StubProtocol.Handler) { StubProtocol.install(handler, id: id) }
    func close() { try? api.auth.logout(); api.transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: id) }
}

@MainActor
private final class ConversationHistoryHarness {
    static let channelID = "11111111-1111-4111-8111-111111111111"
    let store: ConversationStore
    private let harness: ChatHarness
    private let requests = AsyncStream<Void>.makeStream()
    private let responses = AsyncStream<Data>.makeStream()
    private var run: Task<Void, Never>?

    init(mutation: StubProtocol.Handler? = nil) throws {
        harness = try ChatHarness()
        store = ConversationStore(channel: Channel(id: Self.channelID, channel_name: "履歴検証", is_private: false, is_dm: false),
                                  client: harness.client, userID: "33333333-3333-4333-8333-333333333333")
        let requests = requests.continuation
        let responses = responses.stream
        let reads = Mutex(0)
        harness.stub { request in
            if request.url?.path != "/api/shapes/messages" {
                guard let mutation else { throw ZooError.invalidResponse }
                return try await mutation(request)
            }
            requests.yield(())
            var iterator = responses.makeAsyncIterator()
            guard let data = await iterator.next() else { throw CancellationError() }
            let offset = reads.withLock { $0 += 1; return $0 }
            return (200, ["electric-handle": "history", "electric-offset": "0_\(offset)"], data)
        }
    }

    func start() async throws {
        run = Task {
            defer { requests.continuation.finish() }
            await store.run()
        }
        var iterator = requests.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil, "同期処理が要求を開始せずに終了しました。")
    }

    func receive(_ events: [JSONValue]) async throws {
        responses.continuation.yield(try JSONEncoder().encode(events))
        // 次の要求はストアへの反映後に始まるため、固定時間の待機を使わず検証できる。
        var iterator = requests.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil, "同期処理が次の要求を開始せずに終了しました。")
    }

    static func insert(_ message: Message) throws -> JSONValue {
        .object(["key": .string(message.id), "headers": .object(["operation": .string("insert")]),
                 "value": try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(message))])
    }

    static let ready = JSONValue.object(["headers": .object(["control": .string("up-to-date")])])

    func close() {
        run?.cancel()
        responses.continuation.finish()
        requests.continuation.finish()
        harness.close()
    }
}

@MainActor
final class ConversationPerformanceTests: XCTestCase {
    func testLargeHistoryLookups() async throws {
        let harness = try ConversationHistoryHarness()
        defer { harness.close() }
        let messages = (0..<5_000).map { index in
            Message(id: "message-\(index)", user_id: "user", channel_id: ConversationHistoryHarness.channelID,
                    content: "長い履歴の投稿 \(index)", created_at: "2026-11-18T09:00:00Z",
                    parent_id: index.isMultiple(of: 5) ? nil : "message-\(index / 5 * 5)")
        }
        try await harness.start()
        try await harness.receive(messages.map { try ConversationHistoryHarness.insert($0) } + [ConversationHistoryHarness.ready])
        XCTAssertEqual(harness.store.messages.count, 5_000)
        var count = 0
        measure(metrics: [XCTClockMetric()]) {
            for _ in 0..<100 {
                count += harness.store.roots.count
                count += harness.store.replies(to: "message-0").count
            }
        }
        XCTAssertGreaterThan(count, 0)
    }
}

@MainActor
@Suite("会話履歴の更新と索引", .timeLimit(.minutes(1)))
struct ConversationHistoryTests {
    private func message(_ id: String, date: String = "2026-11-18T09:00:00Z", parent: String? = nil) -> Message {
        Message(id: id, user_id: "user", channel_id: ConversationHistoryHarness.channelID,
                content: id, created_at: date, parent_id: parent)
    }

    @Test("親の到着・返信の編集・削除をチャンネルとスレッドの両方に反映する")
    func remoteUpdates() async throws {
        let harness = try ConversationHistoryHarness()
        defer { harness.close() }
        let store = harness.store
        var reply = message("reply", date: "2026-11-18T09:01:00Z", parent: "parent")
        try await harness.start()
        try await harness.receive([ConversationHistoryHarness.insert(reply), ConversationHistoryHarness.ready])
        #expect(store.roots.map(\.id) == ["reply"])
        #expect(store.replies(to: "parent") == [reply])

        try await harness.receive([ConversationHistoryHarness.insert(message("parent"))])
        #expect(store.roots.map(\.id) == ["parent"])
        #expect(store.replies(to: "parent") == [reply])

        reply.content = "編集後の返信"
        reply.updated_at = "2026-11-18T09:02:00Z"
        try await harness.receive([.object(["key": .string("reply"), "headers": .object(["operation": .string("update")]),
                                           "value": .object(["content": .string(reply.content), "updated_at": .string(reply.updated_at!)])])])
        #expect(store.replies(to: "parent") == [reply])
        #expect(store.message(id: "reply") == reply)

        for id in ["parent", "reply"] {
            try await harness.receive([.object(["key": .string(id), "headers": .object(["operation": .string("delete")])])])
            #expect(store.message(id: id) == nil)
            #expect(store.roots == (id == "parent" ? [reply] : []))
        }
        #expect(store.replies(to: "parent").isEmpty)
    }

    @Test("日時順と同時刻のID順を保ち、再同期時には古い索引を破棄する")
    func orderingAndReset() async throws {
        let harness = try ConversationHistoryHarness()
        defer { harness.close() }
        let store = harness.store
        let parent = message("a", date: "2026-11-18T18:00:00+09:00")
        let reply = message("reply", parent: "a")
        var deleted = message("deleted")
        deleted.deleted_at = "2026-11-18T09:01:00Z"
        var foreign = message("foreign")
        foreign.channel_id = "別のチャンネル"
        let rows = [message("late", date: "2026-11-18 09:00:00.123456"), message("b"), parent,
                    message("invalid", date: "日時不明"), reply, deleted, foreign]
        try await harness.start()
        try await harness.receive(rows.map { try ConversationHistoryHarness.insert($0) } + [ConversationHistoryHarness.ready])
        #expect(store.messages.map(\.id) == ["invalid", "a", "b", "reply", "late"])
        #expect(store.roots.map(\.id) == ["invalid", "a", "b", "late"])

        let changed = Mutex(false)
        withObservationTracking {
            _ = store.messages
            _ = store.roots
            _ = store.replies(to: "a")
        } onChange: { changed.withLock { $0 = true } }
        try await harness.receive([ConversationHistoryHarness.ready])
        #expect(!changed.withLock { $0 })

        try await harness.receive([.object(["headers": .object(["control": .string("must-refetch")])])])
        #expect(store.messages.isEmpty)
        #expect(store.roots.isEmpty)
        #expect(store.replies(to: "a").isEmpty)
        #expect(store.message(id: "a") == nil)
        #expect(store.loading)
        try await harness.receive([ConversationHistoryHarness.insert(reply), ConversationHistoryHarness.ready])
        #expect(store.roots == [reply])
        #expect(store.replies(to: "a") == [reply])
    }

    @Test("送信・編集・親の削除と会話終了を索引に反映する", arguments: [false, true])
    func localUpdates(encrypted: Bool) async throws {
        let client = PreviewChatClient()
        let channel = Channel(id: PreviewChatClient.channelID, channel_name: "一般", is_private: false, is_dm: false)
        let store = ConversationStore(channel: channel, client: client, userID: client.currentUser.id)
        store.drafts["root"] = "親投稿"
        await store.send()
        let parent = try #require(store.roots.first)
        store.drafts[parent.id] = "返信"
        await store.send(parentID: parent.id)
        let reply = try #require(store.replies(to: parent.id).first)
        #expect(await store.edit(parent, content: "親投稿を編集"))
        #expect(store.roots.first?.content == "親投稿を編集")
        #expect(await store.edit(reply, content: "返信を編集"))
        let editedReply = try #require(store.replies(to: parent.id).first)
        #expect(editedReply.content == "返信を編集")
        await store.delete(parent)
        #expect(store.roots == [editedReply])
        #expect(store.message(id: parent.id) == nil)

        if encrypted {
            var changed = channel
            changed.encrypted = true
            store.update(changed)
        } else {
            store.shutdown()
        }
        #expect(store.messages.isEmpty)
        #expect(store.roots.isEmpty)
        #expect(store.replies(to: parent.id).isEmpty)
        #expect(store.message(id: reply.id) == nil)
    }
}

@MainActor
@Suite("ネイティブチャット", .timeLimit(.minutes(1)))
struct NativeChatTests {
    private static let channelID = "11111111-1111-4111-8111-111111111111"
    private static let userID = "33333333-3333-4333-8333-333333333333"
    private var channel: Channel { Channel(id: Self.channelID, channel_name: "一般", is_private: false, is_dm: false) }

    nonisolated private static func sentMessage(_ request: URLRequest) throws -> Message {
        let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
        guard case .string(let id) = body["id"], case .string(let content) = body["content"], case .string(let channel) = body["channel_id"] else {
            throw ZooError.invalidResponse
        }
        let parent: String?
        if case .string(let value) = body["parent_id"] { parent = value } else { parent = nil }
        return Message(id: id, user_id: "33333333-3333-4333-8333-333333333333", channel_id: channel, content: content,
                       created_at: "2026-11-18T09:00:00Z", parent_id: parent)
    }

    nonisolated private static func envelope(_ row: some Encodable) throws -> Data {
        let value = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(row))
        return try JSONEncoder().encode(["data": value, "txid": .string("123")])
    }

    @Test("既読の保存失敗を再試行し、成功した位置は重複送信しない")
    func retriesReadSave() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let attempts = Mutex(0)
        let messageID = "22222222-2222-4222-8222-222222222222"
        let readPath = "/api/channels/\(Self.channelID)/read-state"
        harness.stub { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == readPath)
            let body = try JSONDecoder().decode([String: String].self, from: StubProtocol.body(request))
            #expect(body["message_id"] == messageID)
            if attempts.withLock({ $0 += 1; return $0 }) == 1 {
                return (503, [:], Data())
            }
            return (200, [:], Data())
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        var acknowledgements = 0
        store.onRead = { acknowledgements += 1 }
        await store.markRead(through: messageID)
        await store.markRead(through: messageID)
        await store.markRead(through: messageID)
        #expect(attempts.withLock { $0 } == 2)
        #expect(acknowledgements == 1)
        #expect(store.readError == nil)
    }

    @Test("画面を離れた既読リクエストは再試行せず、一覧も更新しない")
    func cancelledReadDoesNotAcknowledge() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let attempts = Mutex(0)
        harness.stub { _ in
            attempts.withLock { $0 += 1 }
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], Data())
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        var acknowledged = false
        store.onRead = { acknowledged = true }
        let task = Task {
            defer { started.continuation.finish() }
            await store.markRead(through: "22222222-2222-4222-8222-222222222222") }
        defer { task.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        task.cancel()
        release.continuation.yield(())
        await task.value
        #expect(attempts.withLock { $0 } == 1)
        #expect(!acknowledged)
        #expect(store.readError == nil)
    }

    @Test("既読の再試行待ちを中断し、同じ会話を開き直すと保存できる")
    func cancelledReadRetryCanResume() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let failed = AsyncStream<Void>.makeStream()
        defer { failed.continuation.finish() }
        let attempts = Mutex(0)
        harness.stub { _ in
            let count = attempts.withLock { $0 += 1; return $0 }
            return (count == 1 ? 503 : 200, [:], Data())
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        var acknowledgements = 0
        store.onRead = { acknowledgements += 1 }
        withObservationTracking { _ = store.readError } onChange: { failed.continuation.yield(()) }
        let messageID = "22222222-2222-4222-8222-222222222222"
        let task = Task {
            defer { failed.continuation.finish() }
            await store.markRead(through: messageID) }
        defer { task.cancel() }
        var iterator = failed.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        #expect(store.readError != nil)
        task.cancel()
        await task.value
        #expect(attempts.withLock { $0 } == 1)
        #expect(acknowledgements == 0)
        await store.markRead(through: messageID)
        #expect(attempts.withLock { $0 } == 2)
        #expect(acknowledgements == 1)
        #expect(store.readError == nil)
    }

    @Test("既読保存の恒久的な拒否は再試行せず、未読を残してエラーを表示する")
    func rejectedReadRemainsUnread() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let attempts = Mutex(0)
        harness.stub { _ in
            attempts.withLock { $0 += 1 }
            return (404, [:], Data())
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        var acknowledged = false
        store.onRead = { acknowledged = true }
        await store.markRead(through: "22222222-2222-4222-8222-222222222222")
        #expect(attempts.withLock { $0 } == 1)
        #expect(!acknowledged)
        #expect(store.readError != nil)
    }

    @Test("一覧取得中に保存した既読を古い応答で巻き戻さず、次回の新着は取得する")
    func staleRefreshDoesNotRestoreUnread() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let reads = Mutex(0)
        let channelData = try JSONEncoder().encode([channel])
        let readData = try JSONEncoder().encode([
            ChannelReadState(channel_id: Self.channelID, has_unread: true, last_read_at: nil)
        ])
        let savedReadData = try JSONEncoder().encode([
            ChannelReadState(channel_id: Self.channelID, has_unread: false, last_read_at: nil)
        ])
        harness.stub { request in
            switch request.url?.path {
            case "/api/channels": return (200, [:], channelData)
            case "/api/shapes/users":
                return (200, ["electric-handle": "users", "electric-offset": "0_1"],
                        Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
            case "/api/channel-read-states":
                let count = reads.withLock { $0 += 1; return $0 }
                if count == 2 {
                    started.continuation.yield(())
                    var iterator = release.stream.makeAsyncIterator()
                    _ = await iterator.next()
                }
                if count == 3 { return (200, [:], savedReadData) }
                return (200, [:], readData)
            default: return (200, [:], Data())
            }
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: Self.userID, user_name: "you", display_name: "あなた"))
        await workspace.refresh()
        #expect(workspace.unread.contains(Self.channelID))
        let refresh = Task {
            defer { started.continuation.finish() }
            await workspace.refresh() }
        defer { refresh.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        await workspace.conversation(for: channel).markRead(through: "22222222-2222-4222-8222-222222222222")
        #expect(!workspace.unread.contains(Self.channelID))
        release.continuation.yield(())
        await refresh.value
        #expect(!workspace.unread.contains(Self.channelID))
        #expect(reads.withLock { $0 } == 3)
        await workspace.refresh()
        #expect(workspace.unread.contains(Self.channelID))
        #expect(reads.withLock { $0 } == 4)
        workspace.shutdown()
    }

    @Test("権限の世代変更で操作を再開し、古い完了で新しい操作のロックを解放しない", arguments: ["送信", "編集", "削除"])
    func operationLocksBelongToAccessGeneration(operation: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Int>.makeStream(), firstRelease = AsyncStream<Void>.makeStream(), secondRelease = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); firstRelease.continuation.finish(); secondRelease.continuation.finish() }
        let requests = Mutex(0)
        let message = Message(id: "22222222-2222-4222-8222-222222222222", user_id: Self.userID, channel_id: Self.channelID,
                              content: "本文", created_at: "2026-11-18T09:00:00Z")
        harness.stub { request in
            let count = requests.withLock { $0 += 1; return $0 }
            started.continuation.yield(count)
            var iterator = (count == 1 ? firstRelease.stream : secondRelease.stream).makeAsyncIterator()
            _ = await iterator.next()
            if operation == "削除" { return (200, [:], Data("{}".utf8)) }
            return (200, [:], try Self.envelope(operation == "送信" ? Self.sentMessage(request) : message))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        func perform() async {
            if operation == "送信" { await store.send() }
            else if operation == "編集" { _ = await store.edit(message, content: "編集") }
            else { await store.delete(message) }
        }
        store.drafts["root"] = "最初の送信"
        let first = Task {
            defer { started.continuation.finish() }
            await perform() }
        defer { first.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() == 1)
        var encrypted = channel
        encrypted.encrypted = true
        store.update(encrypted)
        store.update(channel)
        #expect(store.sending.isEmpty && !store.mutationInProgress)
        store.drafts["root"] = "次の送信"
        let second = Task {
            defer { started.continuation.finish() }
            await perform() }
        defer { second.cancel() }
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while requests.withLock({ $0 }) < 2, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
        try #require(requests.withLock { $0 } == 2, "古い操作の応答を待たずに新しい操作を開始できる")
        firstRelease.continuation.finish()
        await first.value
        #expect(operation == "送信" ? store.sending.contains("root") : store.mutationInProgress)
        secondRelease.continuation.finish()
        await second.value
        #expect(store.sending.isEmpty && !store.mutationInProgress)
        #expect(requests.withLock { $0 } == 2)
    }

    @Test("同一時刻の古い同期行で編集結果を戻さず、取得範囲の変更後も削除結果を保つ", arguments: [false, true])
    func preservesLocalMutationAcrossStaleFeeds(deleting: Bool) async throws {
        let original = Message(id: "22222222-2222-4222-8222-222222222222", user_id: Self.userID, channel_id: Self.channelID,
                               content: "編集前", created_at: "2026-11-18T09:00:00Z", updated_at: "2026-11-18T09:00:00Z",
                               display_data: MessageDisplayData(reply_count: 0))
        var changed = original
        changed.content = "編集後"
        let updated = changed
        let harness = try ConversationHistoryHarness { request in
            #expect(request.httpMethod == (deleting ? "DELETE" : "PATCH"))
            return (200, [:], deleting ? Data("{}".utf8) : try Self.envelope(updated))
        }
        defer { harness.close() }
        try await harness.start()
        try await harness.receive([ConversationHistoryHarness.insert(original), ConversationHistoryHarness.ready])
        if deleting {
            await harness.store.delete(original)
            try await harness.receive([.object(["key": .string(original.id), "headers": .object(["operation": .string("delete")])])])
        } else {
            #expect(await harness.store.edit(original, content: updated.content))
        }
        try await harness.receive([ConversationHistoryHarness.insert(original)])
        #expect(harness.store.message(id: original.id)?.content == (deleting ? nil : updated.content))
        if !deleting {
            var acknowledged = updated
            acknowledged.created_at = "2026-11-18 09:00:00"
            acknowledged.updated_at = "2026-11-18 09:00:00.000+00:00"
            acknowledged.display_data?.reply_count = 3
            let created = try #require(ZooDate.parse(updated.created_at))
            let modifiedText = try #require(updated.updated_at)
            let acknowledgedModifiedText = try #require(acknowledged.updated_at)
            let modified = try #require(ZooDate.parse(modifiedText))
            #expect(try #require(ZooDate.parse(acknowledged.created_at)) == created)
            #expect(try #require(ZooDate.parse(acknowledgedModifiedText)) == modified)
            try await harness.receive([ConversationHistoryHarness.insert(acknowledged)])
            #expect(harness.store.message(id: original.id)?.display_data?.reply_count == 3)
            var newer = updated
            newer.content = "別端末の編集"
            newer.updated_at = "2026-11-18T09:00:01Z"
            try await harness.receive([ConversationHistoryHarness.insert(newer)])
            #expect(harness.store.message(id: original.id)?.content == newer.content)
        }
    }

    @Test("送信失敗後にアクセス世代が変わっても同じIDで再送する", arguments: [false, true])
    func retryIsIdempotent(revoked: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let ids = Mutex<[String]>([])
        harness.stub { request in
            #expect(request.url?.path == "/api/messages")
            #expect(request.httpMethod == "POST")
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            let message = try Self.sentMessage(request)
            let attempt = ids.withLock { $0.append(message.id); return $0.count }
            if attempt == 1 { throw URLError(.networkConnectionLost) }
            return (200, [:], try Self.envelope(message))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.drafts["root"] = "失敗しても消えない本文"
        await store.send()
        #expect(store.drafts["root"] == "失敗しても消えない本文")
        #expect(store.sendErrors["root"] != nil)
        if revoked {
            var encrypted = channel
            encrypted.encrypted = true
            store.update(encrypted)
            store.update(channel)
        }
        await store.send()
        #expect(store.drafts["root"] == "")
        #expect(store.messages.count == 1)
        #expect(store.sendErrors["root"] == nil)
        #expect(ids.withLock { $0.count == 2 && $0[0] == $0[1] })
    }

    @Test("送信中に書き始めた次の下書きを消さない")
    func nextDraftSurvives() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            let message = try Self.sentMessage(request)
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], try Self.envelope(message))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.drafts["root"] = "最初の本文"
        let task = Task {
            defer { started.continuation.finish() }
            await store.send() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        store.drafts["root"] = "次の本文"
        release.continuation.yield(())
        await task.value
        #expect(store.messages.first?.content == "最初の本文")
        #expect(store.drafts["root"] == "次の本文")
        #expect(store.followsFocusedMessageToLatest)
        store.focusedMessageID = store.messages.first?.id
        #expect(!store.followsFocusedMessageToLatest)
    }

    @Test("ログアウト後の送信応答で会話を復元しない")
    func logoutDiscardsLateResponse() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            let message = try Self.sentMessage(request)
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], try Self.envelope(message))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.drafts["root"] = "送信中"
        let task = Task {
            defer { started.continuation.finish() }
            await store.send() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        store.shutdown()
        release.continuation.yield(())
        await task.value
        #expect(store.messages.isEmpty)
        #expect(store.drafts.isEmpty)
    }

    @Test("返信を親メッセージに関連付け、通常の下書きと分離する")
    func replyUsesParent() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let parent = "22222222-2222-4222-8222-222222222222"
        harness.stub { request in
            let message = try Self.sentMessage(request)
            #expect(message.parent_id == parent)
            return (200, [:], try Self.envelope(message))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.drafts["root"] = "通常の本文"
        store.drafts[parent] = "返信の本文"
        await store.send(parentID: parent)
        #expect(store.drafts["root"] == "通常の本文")
        #expect(store.drafts[parent] == "")
        #expect(store.messages.first?.parent_id == parent)
    }

    // 参加不足の 403。`code` を返す前のサーバーは本文だけ。`code` を返すサーバーは、本文が違っても `code` で決まる。
    @Test("参加が必要なスレッドへの返信が403なら、参加して同じ ID で再送する", arguments: [false, true], [
        #"{"error":"Join the thread before replying"}"#,
        #"{"error":"参加してから返信してください","code":"THREAD_JOIN_REQUIRED"}"#
    ])
    func replyRejoinsThreadAfterForbidden(archived: Bool, forbidden: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let parent = "22222222-2222-4222-8222-222222222222"
        let calls = Mutex<[String]>([])
        let queries = Mutex<[String]>([])
        let ids = Mutex<[String]>([])
        harness.stub { request in
            let path = request.url?.path ?? ""
            calls.withLock { $0.append("\(request.httpMethod ?? "") \(path)") }
            if path.hasPrefix("/api/threads/") { queries.withLock { $0.append(request.url?.query ?? "") } }
            if path == "/api/messages" {
                let message = try Self.sentMessage(request)
                let attempt = ids.withLock { $0.append(message.id); return $0.count }
                if attempt == 1 { return (403, [:], Data(forbidden.utf8)) }
                return (200, [:], try Self.envelope(message))
            }
            if path == "/api/threads/\(parent)" {
                let status: JSONValue = .object([
                    "membership": .string("left"), "can_participate": .bool(true),
                    "thread_archive_at": archived ? .string("2026-11-01T00:00:00Z") : .null
                ])
                return (200, [:], try JSONEncoder().encode(["data": status]))
            }
            return (200, [:], try Self.envelope(["ok": true]))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.drafts[parent] = "退出後の返信"
        await store.send(parentID: parent)
        #expect(store.drafts[parent] == "")
        #expect(store.sendErrors[parent] == nil)
        #expect(ids.withLock { $0.count == 2 && $0[0] == $0[1] })
        let expected = ["POST /api/messages", "GET /api/threads/\(parent)"]
            + (archived ? ["POST /api/threads/\(parent)/unarchive"] : [])
            + ["POST /api/threads/\(parent)/join", "POST /api/messages"]
        #expect(calls.withLock { $0 } == expected)
        // 必要なのは参加状態だけ。参加者の一覧（人数に比例する読み取り）は、3つの呼び出しすべてで省く。
        #expect(queries.withLock { $0 } == Array(repeating: "participants=none", count: archived ? 3 : 2))
    }

    @Test("参加不足以外の403（投稿権限なしなど）では参加せず、下書きを保持してエラーにする")
    func otherForbiddenDoesNotJoin() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let parent = "22222222-2222-4222-8222-222222222222"
        let calls = Mutex<[String]>([])
        harness.stub { request in
            calls.withLock { $0.append("\(request.httpMethod ?? "") \(request.url?.path ?? "")") }
            return (403, [:], Data(#"{"error":"Only managers can post in this channel"}"#.utf8))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.drafts[parent] = "送れない返信"
        await store.send(parentID: parent)
        #expect(store.drafts[parent] == "送れない返信")
        #expect(store.sendErrors[parent] != nil)
        #expect(calls.withLock { $0 } == ["POST /api/messages"])
    }

    @Test("403の code が参加不足でなければ、本文が似ていても参加しない", arguments: [
        #"{"error":"Join the thread before replying","code":"POSTING_RESTRICTED"}"#,
        #"{"error":"Join the thread before replying","code":"OTHER"}"#
    ])
    func theCodeWinsOverTheMessage(forbidden: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let parent = "22222222-2222-4222-8222-222222222222"
        let calls = Mutex<[String]>([])
        harness.stub { request in
            calls.withLock { $0.append("\(request.httpMethod ?? "") \(request.url?.path ?? "")") }
            return (403, [:], Data(forbidden.utf8))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.drafts[parent] = "送れない返信"
        await store.send(parentID: parent)
        #expect(store.drafts[parent] == "送れない返信")
        #expect(store.sendErrors[parent] != nil)
        #expect(calls.withLock { $0 } == ["POST /api/messages"])
    }

    @Test("参加不足の403の判定は、code を優先し、code のない応答だけ本文で見る")
    func joinRequiredDecision() throws {
        #expect(ZooError.requiresThreadJoin(code: "THREAD_JOIN_REQUIRED", message: "何でも"))
        #expect(ZooError.requiresThreadJoin(code: nil, message: "Join the thread before replying"))
        #expect(!ZooError.requiresThreadJoin(code: nil, message: "Only managers can post in this channel"))
        #expect(!ZooError.requiresThreadJoin(code: "POSTING_RESTRICTED", message: "Join the thread before replying"))
        let fullMessage = String(repeating: " ", count: 250) + "Join the thread before replying"
        let body = try JSONEncoder().encode(["message": fullMessage])
        let response = try #require(HTTPURLResponse(url: URL(string: "https://api.test.invalid")!, statusCode: 403,
                                                   httpVersion: nil, headerFields: nil))
        do {
            try HTTPTransport.check(body, response)
            Issue.record("参加不足を判定しませんでした。")
        } catch ZooError.threadJoinRequired(let message) {
            #expect(message.count == 240)
        }

    }

    @Test("暗号化・アーカイブ済みチャンネルには平文を送らない", arguments: [true, false])
    func readOnlyChannels(encrypted: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in Issue.record("送信禁止のチャンネルで通信しました。"); throw ZooError.invalidResponse }
        var channel = channel
        channel.encrypted = encrypted
        channel.archive_at = encrypted ? nil : "2026-11-18 09:00:00"
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        store.drafts["root"] = "平文"
        #expect(store.postingRestrictionMessage == (encrypted ? "暗号化された会話には送信できません" : "アーカイブ済みのチャンネルです"))
        await store.send()
        #expect(store.messages.isEmpty)
        #expect(store.drafts["root"] == "平文")
    }

    @Test("認証切れを通知し、再接続の連打を止める")
    func streamUnauthorized() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in (401, [:], Data()) }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        var expired = false
        store.onExpiredSession = { expired = true }
        await store.run()
        #expect(expired)
    }

    @Test("通知のアカウントと遷移先を検証する")
    func notificationRoute() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let channelBytes = try JSONEncoder().encode(channel)
        let user = Person(id: Self.userID, user_name: "you", display_name: "あなた")
        let workspace = WorkspaceStore(client: harness.client, user: user)
        harness.stub { request in
            #expect(request.url?.path == "/api/channels/11111111-1111-4111-8111-111111111111")
            return (200, [:], channelBytes)
        }
        let invalid = try #require(PushDestination(payload: ["user_id": UUID().uuidString, "channel_id": Self.channelID]))
        #expect(await workspace.open(invalid) == false)
        #expect(workspace.path.isEmpty)
        let message = "22222222-2222-4222-8222-222222222222"
        let valid = try #require(PushDestination(payload: ["user_id": Self.userID, "channel_id": Self.channelID, "message_id": message]))
        #expect(await workspace.open(valid))
        #expect(workspace.path == [.channel(Self.channelID)])
        #expect(workspace.conversation(for: channel).focusedMessageID == message)
    }

    @Test("グループDMは宛先全員を送信し、一覧を更新してもメンバーの名前と他タブの履歴を保つ")
    func groupDirectMessage() async throws {
        let preview = PreviewChatClient()
        let workspace = WorkspaceStore(client: preview, user: preview.currentUser)
        await workspace.refresh()
        let recipients = workspace.people.values.filter { $0.id != workspace.user.id }.map(\.id).sorted()
        #expect(recipients.count == 2)
        let dm = try await preview.directMessage(userIDs: recipients)
        workspace.channelPath = [.channel(Self.channelID)]
        workspace.addAndOpen(dm, members: recipients + [workspace.user.id])
        await workspace.refresh()
        #expect(workspace.selectedTab == .directMessages)
        #expect(workspace.dmPath == [.channel(dm.id)])
        #expect(workspace.channelPath == [.channel(Self.channelID)])
        #expect(workspace.name(for: dm) == "あおい、さくら")

        let harness = try ChatHarness()
        defer { harness.close() }
        let bytes = try Self.envelope(dm)
        harness.stub { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/api/dms")
            let payload = try JSONDecoder().decode([String: [String]].self, from: StubProtocol.body(request))
            #expect(payload["target_user_ids"] == recipients)
            return (200, [:], bytes)
        }
        #expect(try await harness.client.directMessage(userIDs: recipients).id == dm.id)
    }

    @Test("通知から会話を取得している途中で別タブに移ると元の要求は画面を切り替えない")
    func leavingInboxCancelsNavigation() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let bytes = try JSONEncoder().encode(channel)
        harness.stub { _ in
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], bytes)
        }
        let user = Person(id: Self.userID, user_name: "you", display_name: "あなた")
        let workspace = WorkspaceStore(client: harness.client, user: user)
        workspace.selectedTab = .notifications
        let open = Task {
            defer { started.continuation.finish() }
            return await workspace.openInboxMessage(channelID: Self.channelID, messageID: nil) }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        workspace.selectedTab = .directMessages
        release.continuation.yield(())
        #expect(await open.value == false)
        #expect(workspace.selectedTab == .directMessages)
        #expect(workspace.channelPath.isEmpty)
    }

    @Test("返信通知は親スレッドを一段だけ開き、チャンネル通知で元の会話に戻る")
    func threadNotificationRoute() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let channelBytes = try JSONEncoder().encode(channel)
        let channelPath = "/api/channels/\(channel.id)"
        harness.stub { request in
            if request.url?.path == "/api/messages" {
                return (200, [:], try Self.envelope(Self.sentMessage(request)))
            }
            #expect(request.url?.path == channelPath)
            return (200, [:], channelBytes)
        }
        let user = Person(id: Self.userID, user_name: "you", display_name: "あなた")
        let workspace = WorkspaceStore(client: harness.client, user: user)
        let store = workspace.conversation(for: channel)
        store.drafts["root"] = "親メッセージ"
        await store.send()
        let parent = try #require(store.roots.first)
        store.drafts[parent.id] = "通知の対象となる返信"
        await store.send(parentID: parent.id)
        let reply = try #require(store.replies(to: parent.id).first)
        let target = try #require(PushDestination(payload: ["user_id": user.id, "channel_id": channel.id, "message_id": reply.id]))
        #expect(await workspace.open(target))
        workspace.openThread(parent.id, in: store, focusComposer: true)
        #expect(workspace.path == [.channel(channel.id), .thread(channelID: channel.id, parentID: parent.id, focusComposer: false)])
        #expect(store.focusedMessageID == reply.id)

        let channelTarget = try #require(PushDestination(payload: ["user_id": user.id, "channel_id": channel.id]))
        #expect(await workspace.open(channelTarget))
        #expect(workspace.path == [.channel(channel.id)])
        #expect(store.focusedMessageID == nil)
    }

    @Test("閲覧権限の喪失後に届く送信応答で履歴を再表示しない")
    func revokedAccessDiscardsLateSend() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let polling = AsyncStream<Void>.makeStream()
        let revoke = AsyncStream<Void>.makeStream()
        let sending = AsyncStream<Void>.makeStream()
        let releaseSend = AsyncStream<Void>.makeStream()
        defer {
            polling.continuation.finish(); revoke.continuation.finish()
            sending.continuation.finish(); releaseSend.continuation.finish()
        }
        let reads = Mutex(0)
        harness.stub { request in
            if request.url?.path == "/api/shapes/messages" {
                if reads.withLock({ $0 += 1; return $0 }) == 1 {
                    return (200, ["electric-handle": "h", "electric-offset": "0_1"], Data(#"[{"key":"old","headers":{"operation":"insert"},"value":{"id":"old","channel_id":"11111111-1111-4111-8111-111111111111","user_id":"u","content":"権限喪失後は表示しない履歴","created_at":"2026-11-18 09:00:00"}},{"headers":{"control":"up-to-date"}}]"#.utf8))
                }
                polling.continuation.yield(())
                var iterator = revoke.stream.makeAsyncIterator()
                _ = await iterator.next()
                return (403, [:], Data())
            }
            let message = try Self.sentMessage(request)
            sending.continuation.yield(())
            var iterator = releaseSend.stream.makeAsyncIterator()
            try #require(await iterator.next() != nil)
            return (200, [:], try Self.envelope(message))
        }
        let store = ConversationStore(channel: channel, client: harness.client, userID: Self.userID)
        let stream = Task {
            defer { polling.continuation.finish() }
            await store.run() }
        defer { stream.cancel() }
        var poll = polling.stream.makeAsyncIterator()
        try #require(await poll.next() != nil)
        #expect(store.messages.count == 1)
        store.drafts["root"] = "権限変更直前の投稿"
        let send = Task {
            defer { sending.continuation.finish() }
            await store.send() }
        var sendStarted = sending.stream.makeAsyncIterator()
        try #require(await sendStarted.next() != nil)
        revoke.continuation.yield(())
        await stream.value
        #expect(store.blocked)
        #expect(store.postingRestrictionMessage == "このチャンネルには送信できません")
        #expect(store.messages.isEmpty)
        releaseSend.continuation.yield(())
        await send.value
        #expect(store.messages.isEmpty)
        #expect(!store.canPost)
    }

    @Test("古い一覧応答で作成直後のチャンネルと画面を消さない")
    func staleRefreshCannotRemoveCreatedChannel() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let waiting = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { waiting.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            if request.url?.path == "/api/shapes/users" {
                waiting.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                return (200, ["electric-handle": "users", "electric-offset": "0_1"],
                        Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
            }
            return (200, [:], Data("[]".utf8))
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: Self.userID, user_name: "you", display_name: "あなた"))
        let refresh = Task {
            defer { waiting.continuation.finish() }
            await workspace.refresh() }
        var iterator = waiting.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        workspace.addAndOpen(channel)
        release.continuation.yield(())
        await refresh.value
        #expect(workspace.channels.map(\.id) == [Self.channelID])
        #expect(workspace.path == [.channel(Self.channelID)])
    }

    @Test("一覧に含まれない bot の投稿者を個別 API で補完する")
    func resolvesBotAuthors() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let botID = "88888888-8888-4888-8888-888888888888"
        let bot = Person(id: botID, user_name: "bot", display_name: "お知らせ係", is_bot: true)
        let body = try JSONEncoder().encode(bot)
        harness.stub { request in
            #expect(request.url?.path == "/api/users/\(botID)")
            return (200, [:], body)
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: Self.userID, user_name: "you", display_name: "あなた"))
        let message = Message(id: UUID().uuidString, user_id: botID, channel_id: Self.channelID, content: "お知らせ", created_at: "2026-11-18 09:00:00")
        await workspace.resolveAuthors(in: [message])
        #expect(workspace.people[botID]?.name == "お知らせ係")
        #expect(workspace.people[botID]?.is_bot == true)
    }

    @Test("Web と同じ users shape を最後まで取得し、通常ユーザーと bot の画像を保持する")
    func usersShapeIncludesAvatars() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let requests = Mutex(0)
        harness.stub { request in
            #expect(request.url?.path == "/api/shapes/users")
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            let url = try #require(request.url)
            let query = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems)
            #expect(!query.contains { $0.name == "live" })
            let page = requests.withLock { $0 += 1; return $0 }
            if page == 1 {
                return (409, ["electric-handle": "users-new"], Data())
            }
            #expect(query.contains { $0.name == "handle" && $0.value == "users-new" })
            if page == 2 {
                #expect(query.contains { $0.name == "offset" && $0.value == "-1" })
                return (200, ["electric-handle": "users-new", "electric-offset": "0_1"], Data(#"[{"key":"u","headers":{"operation":"insert"},"value":{"id":"user","user_name":"sakura","display_name":"さくら","is_bot":"false","avatar_url":"https://avatars.test.invalid/user.png"}}]"#.utf8))
            }
            #expect(page == 3)
            #expect(query.contains { $0.name == "offset" && $0.value == "0_1" })
            return (200, ["electric-handle": "users-new", "electric-offset": "0_2"], Data(#"[{"key":"b","headers":{"operation":"insert"},"value":{"id":"bot","user_name":"bot","display_name":"お知らせ係","is_bot":"true","avatar_url":"https://avatars.test.invalid/bot.png"}},{"headers":{"control":"up-to-date"}}]"#.utf8))
        }
        let people = try await harness.client.people()
        #expect(people.count == 2)
        #expect(people.first { $0.id == "user" }?.avatarURL?.absoluteString == "https://avatars.test.invalid/user.png")
        #expect(people.first { $0.id == "bot" }?.avatarURL?.absoluteString == "https://avatars.test.invalid/bot.png")
        #expect(requests.withLock { $0 } == 3)
    }

    @Test("ユーザー再取得で画像の変更と削除を反映し、DM の相手にも同じ画像を渡す")
    func avatarRefresh() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let user = Person(id: Self.userID, user_name: "you", display_name: "あなた")
        let peerID = "44444444-4444-4444-8444-444444444444"
        let dm = Channel(id: Self.channelID, channel_name: "dm:\(Self.userID):\(peerID)", is_private: true, is_dm: true)
        let workspace = WorkspaceStore(client: harness.client, user: user)
        for avatar in ["https://avatars.test.invalid/first.png", "https://avatars.test.invalid/second.png", nil] {
            let peer = Person(id: peerID, user_name: "sakura", display_name: "さくら", avatar_url: avatar)
            let value = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(peer))
            let data = try JSONEncoder().encode([
                JSONValue.object(["key": .string(peerID), "headers": .object(["operation": .string("insert")]), "value": value]),
                .object(["headers": .object(["control": .string("up-to-date")])])
            ])
            harness.stub { request in
                if request.url?.path == "/api/shapes/users" {
                    return (200, ["electric-handle": "users", "electric-offset": "0_1"], data)
                }
                return (200, [:], Data("[]".utf8))
            }
            await workspace.refresh()
            #expect(workspace.error == nil)
            #expect(try #require(workspace.people[peerID]).avatarURL?.absoluteString == avatar)
            #expect(try #require(workspace.directMessagePeer(for: dm)).avatarURL?.absoluteString == avatar)
        }
    }
}

@Suite("ユーザーアイコンの URL")
struct AvatarModelTests {
    @Test("REST と Electric のユーザー種別を読める", arguments: ["true", "false", #""true""#, #""false""#, #""t""#, #""f""#])
    func userKind(raw: String) throws {
        let json = #"{"id":"u","user_name":"user","display_name":"ユーザー","is_bot":"# + raw + "}"
        let person = try JSONDecoder().decode(Person.self, from: Data(json.utf8))
        #expect(person.is_bot == ["true", #""true""#, #""t""#].contains(raw))
    }

    @Test("画像の省略・null を含むユーザー応答を読める", arguments: [
        "", #", "avatar_url": null"#, #", "avatar_url": "https://avatars.test.invalid/a.png""#
    ])
    func optionalAvatar(field: String) throws {
        let json = #"{"id":"u","user_name":"user","display_name":"ユーザー","is_bot":false"# + field + "}"
        let person = try JSONDecoder().decode(Person.self, from: Data(json.utf8))
        #expect(person.avatarURL?.absoluteString == (field.contains("https://") ? "https://avatars.test.invalid/a.png" : nil))
    }

    @Test("不正・非 HTTPS・認証情報付きの画像 URL を読み込まない", arguments: [
        "", "/avatar.png", "http://avatars.test.invalid/a.png", "file:///tmp/avatar.png", "data:image/png;base64,AAAA",
        "https:///", "https://user:secret@avatars.test.invalid/a.png", "https://avatars.test.invalid/" + String(repeating: "a", count: 2048)
    ])
    func invalidAvatar(raw: String) {
        let person = Person(id: "u", user_name: "user", display_name: "ユーザー", avatar_url: raw)
        #expect(person.avatarURL == nil)
    }
}
