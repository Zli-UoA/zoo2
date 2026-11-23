import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor
@Suite("チャンネルの編集とアーカイブ")
struct ChannelManagementTests {
    private let user = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")
    private var channel: Channel {
        Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "一般", description: "元の説明",
                is_private: false, is_dm: false, created_user_id: user.id, updated_at: "2026-11-20T00:00:00Z")
    }

    nonisolated private static func envelope(_ value: some Encodable) throws -> Data {
        let data = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(value))
        return try JSONEncoder().encode(["data": data])
    }

    @Test("APIとキャッシュの復元で作成者と更新時刻を保持する")
    func channelRoundTrip() throws {
        let restored = try JSONDecoder().decode(Channel.self, from: JSONEncoder().encode(channel))
        #expect(restored.created_user_id == user.id)
        #expect(restored.updated_at == channel.updated_at)
        #expect(restored.canManage(by: user))
    }

    @Test("作成者と管理者だけが通常チャンネルを管理できる", arguments: ["owner", "admin", "other", "dm", "archived", "deactivated"])
    func permissions(kind: String) {
        var channel = channel
        var user = user
        if kind == "admin" || kind == "other" { channel.created_user_id = "other" }
        if kind == "admin" { user.is_admin = true }
        if kind == "dm" { channel.is_dm = true }
        if kind == "archived" { channel.archive_at = "2026-11-20T00:00:00Z" }
        if kind == "deactivated" { user.is_deactivated = true }
        #expect(channel.canManage(by: user) == (kind == "owner" || kind == "admin"))
    }

    @Test("編集とアーカイブの応答を開いている会話に反映し、古い同期では巻き戻さない")
    func writesAndStaleRefresh() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let initial = channel
        let incoming = Mutex(initial)
        let writes = Mutex<[String]>([])
        harness.stub { request in
            if request.httpMethod == "PATCH" {
                #expect(request.url?.path == "/api/channels/\(initial.id)")
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body == ["channel_name": .string("変更済み"), "description": .string("")])
                writes.withLock { $0.append("edit") }
                var updated = initial
                updated.channel_name = "変更済み"; updated.description = ""; updated.updated_at = "2026-11-20T00:00:02Z"
                return (200, [:], try Self.envelope(updated))
            }
            if request.httpMethod == "POST" {
                #expect(request.url?.path == "/api/channels/\(initial.id)/archive")
                writes.withLock { $0.append("archive") }
                var updated = initial
                updated.channel_name = "変更済み"; updated.description = ""
                updated.archive_at = "2026-11-20T00:00:03Z"; updated.updated_at = updated.archive_at
                return (200, [:], try Self.envelope(updated))
            }
            switch request.url?.path {
            case "/api/channels/\(initial.id)":
                var updated = initial
                updated.channel_name = "変更済み"; updated.description = ""
                updated.archive_at = "2026-11-20T00:00:03Z"; updated.updated_at = updated.archive_at
                return (200, [:], try JSONEncoder().encode(updated))
            case "/api/channels": return (200, [:], try JSONEncoder().encode([incoming.withLock { $0 }]))
            case "/api/channel-read-states": return (200, [:], Data("[]".utf8))
            case "/api/shapes/users":
                return (200, ["electric-handle": "users", "electric-offset": "0_1"], Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
            default: Issue.record("想定外の通信です"); return (500, [:], Data())
            }
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        workspace.addAndOpen(initial)
        let conversation = workspace.conversation(for: initial)
        conversation.drafts["root"] = "下書き"
        try await workspace.updateChannel(initial, name: "変更済み", description: "")
        #expect(conversation.channel.channel_name == "変更済み")
        #expect(conversation.drafts["root"] == "下書き")
        incoming.withLock { $0.is_private = true }
        await workspace.refresh()
        #expect(workspace.error == nil)
        #expect(workspace.channels.first?.channel_name == "変更済み")
        #expect(workspace.channels.first?.description == "")
        #expect(workspace.channels.first?.updated_at == "2026-11-20T00:00:02Z")
        #expect(workspace.channels.first?.is_private == true)
        #expect(conversation.channel.is_private == true)
        try await workspace.archiveChannel(try #require(workspace.channels.first))
        #expect(await workspace.openMessage(channelID: initial.id, messageID: nil))
        await workspace.refresh()
        #expect(!conversation.canPost)
        #expect(workspace.channels.first?.archive_at != nil)
        incoming.withLock { $0.channel_name = "別の端末で変更"; $0.updated_at = "2026-11-20T00:00:04Z" }
        await workspace.refresh()
        #expect(workspace.channels.first?.channel_name == "別の端末で変更")
        #expect(conversation.canPost)
        #expect(writes.withLock { $0 } == ["edit", "archive"])
    }

    @Test("一覧がアーカイブを確認した後に届く単件応答は、更新時刻が新しい場合だけ反映する",
          arguments: [false, true], [false, true])
    func delayedOpenAfterConfirmedArchive(newerResponse: Bool, initiallyUpdated: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        var original = channel
        if !initiallyUpdated { original.updated_at = nil }
        let initial = original
        var archived = initial
        archived.archive_at = "2026-11-20T00:00:03Z"
        archived.updated_at = archived.archive_at
        let confirmed = archived
        var response = initial
        if newerResponse {
            response.channel_name = "別の端末で再開"
            response.updated_at = "2026-11-20T00:00:04Z"
        }
        let fetched = response
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            switch request.url?.path {
            case "/api/channels/\(initial.id)":
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                return (200, [:], try JSONEncoder().encode(fetched))
            case "/api/channels/\(initial.id)/archive":
                return (200, [:], try Self.envelope(confirmed))
            case "/api/channels": return (200, [:], try JSONEncoder().encode([confirmed]))
            case "/api/channel-read-states": return (200, [:], Data("[]".utf8))
            case "/api/shapes/users":
                return (200, ["electric-handle": "users", "electric-offset": "0_1"], Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
            default: Issue.record("想定外の通信です"); return (500, [:], Data())
            }
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        workspace.addAndOpen(initial)
        let conversation = workspace.conversation(for: initial)
        conversation.drafts["root"] = "下書き"
        let opening = Task {
            defer { started.continuation.finish() }
            return await workspace.openMessage(channelID: initial.id, messageID: nil)
        }
        defer { opening.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        try await workspace.archiveChannel(initial)
        await workspace.refresh()
        try #require(workspace.error == nil && workspace.channels.first == confirmed)
        release.continuation.finish()
        #expect(await opening.value)
        let expected = newerResponse ? fetched : confirmed
        #expect(workspace.channels.first == expected)
        #expect(conversation.channel == expected)
        #expect(conversation.canPost == newerResponse)
        #expect(conversation.drafts["root"] == "下書き")
    }

    @Test("名前を変えない説明編集では予約名も送信し直さず、失敗後は元の表示を保つ")
    func descriptionOnlyFailure() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        var channel = channel
        channel.channel_name = "general"
        harness.stub { request in
            let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
            #expect(body == ["description": .string("説明だけ変更")])
            return (403, [:], Data())
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        workspace.addAndOpen(channel)
        await #expect(throws: ZooError.self) { try await workspace.updateChannel(channel, name: "general", description: "説明だけ変更") }
        #expect(workspace.channels.first?.description == "元の説明")
        #expect(workspace.updatingChannels.isEmpty)
    }

    @Test("ログアウト後に届く編集応答は状態を復活させない")
    func lateResponseAfterLogout() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let initial = channel
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { _ in
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            try #require(await iterator.next() != nil)
            return (200, [:], try Self.envelope(initial))
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        workspace.addAndOpen(initial)
        let write = Task {
            defer { started.continuation.finish() }
            return try await workspace.updateChannel(initial, name: "変更", description: "")
        }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        workspace.shutdown()
        release.continuation.yield(())
        await #expect(throws: CancellationError.self) { try await write.value }
        #expect(workspace.channels.isEmpty)
    }

    @Test("編集・アーカイブ中の権限失効後に古い応答を返さない", arguments: [false, true])
    func lateResponseAfterRevocation(archive: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let cache = try await SyncCache.open(inMemory: true)
        let lease = try await cache.activate(connection: SyncCache.connectionKey(harness.api.settings), user: user)
        let sync = SyncSession(cache: cache, lease: lease)
        let initial = channel
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { _ in
            started.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            try #require(await iterator.next() != nil)
            return (200, [:], try Self.envelope(initial))
        }
        let client = NativeChatClient(api: harness.api, sync: sync)
        let pending = Task {
            defer { started.continuation.finish() }
            if archive { return try await client.archiveChannel(id: initial.id) }
            return try await client.updateChannel(id: initial.id, name: "変更", description: "")
        }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        try await sync.revokeChannel(initial.id)
        release.continuation.yield(())
        await #expect(throws: CancellationError.self) { _ = try await pending.value }
        try await sync.close().value
    }
}
