import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor
@Suite("チャンネルの通知設定", .timeLimit(.minutes(1)))
struct ChannelNotificationTests {
    private let channelID = "11111111-1111-4111-8111-111111111111"

    @Test("通知設定の取得と保存は共有APIを使う", arguments: ChannelNotificationLevel.allCases)
    func api(level: ChannelNotificationLevel) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let channelID = channelID
        let requests = Mutex<[String]>([])
        harness.stub { request in
            #expect(request.url?.path == "/api/channels/\(channelID)/notification-settings")
            let method = request.httpMethod ?? ""
            requests.withLock { $0.append(method) }
            if method == "PATCH" {
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body == ["level": .string(level.rawValue)])
                return (200, [:], Data("{\"data\":{\"level\":\"\(level.rawValue)\"},\"txid\":null}".utf8))
            }
            return (200, [:], Data(#"{"level":"mentions"}"#.utf8))
        }
        let store = ChannelNotifications(client: harness.client, userID: "me")
        await store.load(channelID)
        #expect(store.level(for: channelID) == .mentions)
        await store.save(level, channelID: channelID)
        #expect(store.level(for: channelID) == level)
        #expect(store.errors.isEmpty)
        #expect(requests.withLock { $0 } == ["GET", "PATCH"])
    }

    @Test("本人の専用Shapeだけを購読し、キャンセルで通信を終える")
    func subscription() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let requests = Mutex(0)
        harness.stub { request in
            #expect(request.url?.path == "/api/shapes/channel_notification_settings")
            #expect(request.url?.query?.contains("channel_id") == false)
            requests.withLock { $0 += 1 }
            return (200, ["electric-handle": "settings", "electric-offset": "1_0"], Data(#"[{"key":"setting","headers":{"operation":"insert","txids":[13]},"value":{"user_id":"me","channel_id":"c","notification_level":"all"}},{"key":"setting","headers":{"operation":"update","txids":[14]},"value":{"notification_level":"mute"}},{"headers":{"control":"up-to-date"}}]"#.utf8))
        }
        let store = ChannelNotifications(client: harness.client, userID: "me")
        let received = AsyncStream<Void>.makeStream()
        defer { received.continuation.finish() }
        let task = Task {
            try await harness.client.observeChannelNotificationSettings { snapshot in
                guard let rows = snapshot.rows else { return }
                #expect(snapshot.transactionIDs == [13, 14])
                #expect(snapshot.freshSnapshot)
                store.receive(rows, transactionIDs: snapshot.transactionIDs, freshSnapshot: snapshot.freshSnapshot)
                received.continuation.yield(())
            }
        }
        defer { task.cancel() }
        var iterator = received.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(store.level(for: "c") == .mute)
        task.cancel()
        _ = await task.result
        #expect(requests.withLock { $0 } == 1)
    }

    @Test("別ユーザーを除外し、設定のないチャンネルはメンションを使う")
    func accountScope() {
        let store = ChannelNotifications(client: PreviewChatClient(), userID: "me")
        store.receive([.init(user_id: "other", channel_id: channelID, notification_level: .mute)])
        #expect(store.level(for: channelID) == .mentions)
        store.receive([.init(user_id: "me", channel_id: channelID, notification_level: .all)])
        #expect(store.level(for: channelID) == .all)
        store.receive([])
        #expect(store.level(for: channelID) == .mentions)
    }

    @Test("保存に失敗しても現在値を保持し、再試行できる")
    func failedSave() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let fail = Mutex(true)
        harness.stub { request in
            #expect(request.httpMethod == "PATCH")
            if fail.withLock({ $0 }) { return (503, [:], Data(#"{"error":"保存できません"}"#.utf8)) }
            return (200, [:], Data(#"{"data":{"level":"mute"},"txid":"12"}"#.utf8))
        }
        let store = ChannelNotifications(client: harness.client, userID: "me")
        store.receive([.init(user_id: "me", channel_id: channelID, notification_level: .all)])
        await store.save(.mute, channelID: channelID)
        #expect(store.level(for: channelID) == .all)
        #expect(store.errors[channelID] != nil)
        #expect(store.saving.isEmpty)
        fail.withLock { $0 = false }
        await store.save(.mute, channelID: channelID)
        #expect(store.level(for: channelID) == .mute)
        #expect(store.errors.isEmpty)
        store.receive([.init(user_id: "me", channel_id: channelID, notification_level: .all)])
        #expect(store.level(for: channelID) == .mute)
        store.receive([.init(user_id: "me", channel_id: channelID, notification_level: .mute)], transactionIDs: [12])
        store.receive([.init(user_id: "me", channel_id: channelID, notification_level: .mentions)])
        #expect(store.level(for: channelID) == .mentions)
    }

    @Test("取得中の重複要求をまとめ、遅れた取得でShapeの変更を戻さない")
    func delayedLoad() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let requests = Mutex(0)
        harness.stub { _ in
            requests.withLock { $0 += 1 }
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], Data(#"{"level":"mentions"}"#.utf8))
        }
        let store = ChannelNotifications(client: harness.client, userID: "me")
        let load = Task { await store.load(channelID) }
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        await store.load(channelID)
        await store.save(.mute, channelID: channelID)
        store.receive([.init(user_id: "me", channel_id: channelID, notification_level: .all)])
        release.continuation.finish()
        await load.value
        #expect(store.level(for: channelID) == .all)
        #expect(requests.withLock { $0 } == 1)
    }

    @Test("ログアウト中に届く保存応答は破棄する")
    func shutdownDuringSave() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { _ in
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], Data(#"{"data":{"level":"all"},"txid":"13"}"#.utf8))
        }
        let store = ChannelNotifications(client: harness.client, userID: "me")
        let saving = Task { await store.save(.all, channelID: channelID) }
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        await store.save(.mute, channelID: channelID)
        store.shutdown()
        release.continuation.finish()
        await saving.value
        #expect(store.level(for: channelID) == nil)
        #expect(store.errors.isEmpty)
    }

    @Test("中間値を飛ばした同期や再取得を遅い保存応答で戻さない", arguments: [false, true])
    func delayedSaveResponse(fresh: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { _ in
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], Data(#"{"data":{"level":"all"},"txid":"13"}"#.utf8))
        }
        let store = ChannelNotifications(client: harness.client, userID: "me")
        store.receive([])
        let saving = Task { await store.save(.all, channelID: channelID) }
        var iterator = started.stream.makeAsyncIterator()
        _ = await iterator.next()
        store.receive([.init(user_id: "me", channel_id: channelID, notification_level: .mute)],
                      transactionIDs: fresh ? [] : [13, 14], freshSnapshot: fresh)
        release.continuation.finish()
        await saving.value
        #expect(store.level(for: channelID) == .mute)
    }

    @Test("行なしのメンション設定を同値保存した後も別端末の変更に追従する")
    func mentionsNoOp() async throws {
        let client = PreviewChatClient()
        let store = ChannelNotifications(client: client, userID: client.currentUser.id)
        store.receive([])
        await store.save(.mentions, channelID: channelID)
        store.receive([.init(user_id: client.currentUser.id, channel_id: channelID, notification_level: .all)])
        #expect(store.level(for: channelID) == .all)
    }

    @Test("保存応答後のまとめられた変更と再接続の最新値へ追従する", arguments: [false, true])
    func coalescedChangesAfterResponse(fresh: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in
            (200, [:], Data(#"{"data":{"level":"all"},"txid":"13"}"#.utf8))
        }
        let store = ChannelNotifications(client: harness.client, userID: "me")
        store.receive([])
        await store.save(.all, channelID: channelID)
        #expect(store.level(for: channelID) == .all)
        store.receive([.init(user_id: "me", channel_id: channelID, notification_level: .mute)],
                      transactionIDs: fresh ? [] : [13, 14], freshSnapshot: fresh)
        #expect(store.level(for: channelID) == .mute)
    }
}
