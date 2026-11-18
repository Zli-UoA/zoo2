import Foundation
import Testing
@testable import Zoo

@MainActor
@Suite("スタンプの利用履歴")
struct RecentEmojiTests {
    @Test("新しい利用を先頭に移し、重複を除き、16件まで永続化する")
    func persistenceAndLimit() throws {
        let suite = "recent-emoji-tests-" + UUID().uuidString
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let history = RecentEmojiStore(server: "https://example.com/", userID: "USER", defaults: defaults)
        for n in 0..<20 { history.record(":stamp\(n):") }
        history.record(":stamp10:")
        #expect(history.values.count == 16)
        #expect(history.values.first == ":stamp10:")
        #expect(Set(history.values).count == 16)
        #expect(!history.values.contains(":stamp3:"))
        let restored = RecentEmojiStore(server: "https://example.com", userID: "user", defaults: defaults)
        #expect(restored.values == history.values)
        #expect(RecentEmojiStore(server: "https://other.com", userID: "user", defaults: defaults).values.isEmpty)
        #expect(RecentEmojiStore(server: "https://example.com", userID: "other", defaults: defaults).values.isEmpty)
    }

    @Test("履歴を優先し、別名をまとめ、不足分を既定候補で補う")
    func choices() {
        let history = RecentEmojiStore(server: "s", userID: "u", defaults: nil)
        let fallback = ["👍", "❤️", "🎉", "😊", "😢", "👏"]
        #expect(history.choices(fallback: fallback, canonical: { $0 }) == fallback)
        history.record(":hana:")
        history.record(":sakura:")
        history.record("👏")
        #expect(history.choices(fallback: fallback, canonical: { $0 == ":hana:" ? ":sakura:" : $0 }) ==
                ["👏", ":sakura:", "👍", "❤️", "🎉", "😊"])
    }

    @Test("追加できない外部スタンプや不正な値を履歴に入れない")
    func invalidValues() {
        let history = RecentEmojiStore(server: "s", userID: "u", defaults: nil)
        for value in ["", "apub:external:name", String(repeating: "x", count: 257)] { history.record(value) }
        #expect(history.values.isEmpty)
    }

    @Test("チャンネルとスレッドの追加成功を共有し、取り消しでは順序を変えない")
    func sharedHistory() async throws {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        let channel = try await client.channel(id: PreviewChatClient.channelID)
        let store = workspace.conversation(for: channel)
        store.drafts["root"] = "本文"
        await store.send()
        let root = try #require(store.roots.last)
        await store.toggleReaction(messageID: root.id, emoji: "👍")
        store.drafts[root.id] = "返信"
        await store.send(parentID: root.id)
        let reply = try #require(store.replies(to: root.id).first)
        await store.toggleReaction(messageID: reply.id, emoji: "🎉")
        #expect(workspace.recentEmojis.values == ["🎉", "👍"])
        await store.toggleReaction(messageID: root.id, emoji: "👍")
        #expect(workspace.recentEmojis.values == ["🎉", "👍"])
    }
}
