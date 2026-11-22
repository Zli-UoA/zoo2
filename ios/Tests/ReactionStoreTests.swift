import Foundation
import Testing
@testable import Zoo

@MainActor
@Suite("会話のスタンプ", .timeLimit(.minutes(1)))
struct ReactionStoreTests {
    private let channel = Channel(id: UUID().uuidString, channel_name: "一般", is_private: false, is_dm: false)
    private let messageID = UUID().uuidString
    private let userID = UUID().uuidString

    private func seed(_ harness: ChatHarness, rows: [MessageReaction]?) async throws -> ConversationStore {
        let display = rows.map { MessageDisplayData(reactions: $0) }
        let message = Message(id: messageID, user_id: userID, channel_id: channel.id, content: "本文",
                              created_at: "2026-11-18T09:00:00Z", display_data: display)
        let data = try JSONEncoder().encode(["data": message])
        harness.stub { _ in (200, [:], data) }
        let store = ConversationStore(channel: channel, client: harness.client, userID: userID)
        store.drafts["root"] = message.content
        await store.send()
        try #require(store.messages.first?.id == messageID)
        return store
    }

    @Test("外部の利用者を含めて人数を数え、別名での取り消しには保存された名前を送る")
    func groupAndRemove() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let mine = MessageReaction(id: "mine", message_id: messageID, user_id: userID, emoji: ":hana:")
        let peer = MessageReaction(id: "peer", message_id: messageID, user_id: "https://social.invalid/actor", emoji: ":sakura:")
        let store = try await seed(harness, rows: [mine, peer, peer])
        store.canonicalEmoji = { $0 == ":hana:" ? ":sakura:" : $0 }
        #expect(!store.needsReactionStream)
        #expect(store.reactions(for: messageID) == [ReactionGroup(emoji: ":sakura:", count: 2, isMine: true)])
        harness.stub { request in
            #expect(request.httpMethod == "DELETE")
            let url = try #require(request.url)
            #expect(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first?.value == ":hana:")
            return (200, [:], Data())
        }
        await store.toggleReaction(messageID: messageID, emoji: ":sakura:")
        #expect(store.reactionError(for: messageID) == nil)
        #expect(store.reactions(for: messageID) == [ReactionGroup(emoji: ":sakura:", count: 1, isMine: false)])
    }

    @Test("変更失敗時の表示を保ち、同じスタンプの連打をまとめ、ログアウト後の応答を破棄する")
    func failedAndLateMutations() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let store = try await seed(harness, rows: [])
        var additions: [String] = []
        store.onReactionAdded = { additions.append($0) }
        harness.stub { _ in (503, [:], Data()) }
        await store.toggleReaction(messageID: messageID, emoji: "👍")
        #expect(store.reactions(for: messageID).isEmpty)
        #expect(store.reactionError(for: messageID) != nil)
        #expect(additions.isEmpty)
        let entered = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        let row = MessageReaction(id: "mine", message_id: messageID, user_id: userID, emoji: "👍")
        let data = try JSONEncoder().encode(["data": row])
        harness.stub { _ in
            entered.continuation.yield(())
            var iterator = release.stream.makeAsyncIterator()
            _ = await iterator.next()
            return (200, [:], data)
        }
        let write = Task { await store.toggleReaction(messageID: messageID, emoji: "👍") }
        var iterator = entered.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(store.isReactionPending(messageID: messageID, emoji: "👍"))
        await store.toggleReaction(messageID: messageID, emoji: "👍")
        store.shutdown()
        release.continuation.finish()
        await write.value
        #expect(store.reactions(for: messageID).isEmpty)
        #expect(!store.isReactionPending(messageID: messageID, emoji: "👍"))
        #expect(!store.canReact(to: messageID))
        #expect(additions.isEmpty)
    }

    @Test("旧形式では同期前の書き込みを防ぎ、アーカイブ後も取得済みのスタンプを表示する")
    func fallbackAndArchive() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let store = try await seed(harness, rows: nil)
        #expect(store.needsReactionStream)
        #expect(!store.canReact(to: messageID))
        let row = MessageReaction(id: "mine", message_id: messageID, user_id: userID, emoji: "👏")
        let raw = String(decoding: try JSONEncoder().encode(row), as: UTF8.self)
        let events = #"[{"key":"mine","headers":{"operation":"insert"},"value":\#(raw)},{"headers":{"control":"up-to-date"}}]"#
        let polling = AsyncStream<Void>.makeStream()
        harness.stub { request in
            if request.url?.query?.contains("live=true") == true {
                polling.continuation.yield(())
                try await Task.sleep(for: .seconds(10))
            }
            return (200, ["electric-offset": "0_1", "electric-handle": "reactions"], Data(events.utf8))
        }
        let stream = Task { await store.runReactions() }
        var iterator = polling.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(store.canReact(to: messageID))
        var archived = channel
        archived.archive_at = "2026-11-18"
        store.update(archived)
        #expect(!store.canReact(to: messageID))
        #expect(store.reactions(for: messageID) == [ReactionGroup(emoji: "👏", count: 1, isMine: true)])
        stream.cancel()
        await stream.value
    }

    @Test("旧形式のスタンプ購読で権限を失ったら、本文とスタンプの両方を閉じる", arguments: [403, 404])
    func revokedByReactionSubscription(status: Int) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let store = try await seed(harness, rows: nil)
        harness.stub { _ in (status, [:], Data()) }
        await store.runReactions()
        #expect(store.blocked)
        #expect(store.messages.isEmpty)
        #expect(!store.canPost)
        #expect(!store.needsReactionStream)
    }

    @Test("他の端末で直後に取り消されたスタンプは、新しい初期取得が成功してから表示を戻す")
    func freshSnapshotResolvesUnobservedWrite() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let store = try await seed(harness, rows: [])
        let row = MessageReaction(id: "mine", message_id: messageID, user_id: userID, emoji: "👍")
        let data = try JSONEncoder().encode(["data": row])
        harness.stub { _ in (200, [:], data) }
        await store.toggleReaction(messageID: messageID, emoji: "👍")
        #expect(store.reactions(for: messageID).first?.isMine == true)
        #expect(store.reactionResyncTicket != nil)
        let original = try #require(store.messages.first)
        let raw = String(decoding: try JSONEncoder().encode(original), as: UTF8.self)
        let events = #"[{"key":"message","headers":{"operation":"insert"},"value":\#(raw)},{"headers":{"control":"up-to-date"}}]"#
        let polling = AsyncStream<Void>.makeStream()
        harness.stub { request in
            if request.url?.query?.contains("live=true") == true {
                polling.continuation.yield(())
                try await Task.sleep(for: .seconds(10))
            }
            return (200, ["electric-offset": "0_1", "electric-handle": "messages"], Data(events.utf8))
        }
        let stream = Task { await store.run() }
        var iterator = polling.stream.makeAsyncIterator()
        _ = await iterator.next()
        #expect(store.reactions(for: messageID).isEmpty)
        #expect(store.reactionResyncTicket == nil)
        stream.cancel()
        await stream.value
    }
}
