import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor
@Suite("Siriからのメッセージと通話", .timeLimit(.minutes(1)))
struct ShortcutActionsTests {
    private let user = Person(id: "33333333-3333-4333-8333-333333333333", user_name: "you", display_name: "あなた")
    private let peer = Person(id: "44444444-4444-4444-8444-444444444444", user_name: "sakura", display_name: "さくら")
    private let channel = Channel(id: "66666666-6666-4666-8666-666666666666", channel_name: "dm:you:sakura", is_private: true, is_dm: true)

    private func recipient(_ workspace: WorkspaceStore) -> ShortcutRecipient {
        .init(id: "\(workspace.client.serverURL)|\(workspace.user.id)|\(peer.id)", personID: peer.id, name: peer.name, username: peer.user_name)
    }

    nonisolated private static func envelope(_ value: some Encodable) throws -> Data {
        let value = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(value))
        return try JSONEncoder().encode(["data": value])
    }

    @Test("同名メンバーを勝手に一人へ絞らず、自分・bot・無効なメンバーを除く")
    func recipientCandidates() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        var other = peer; other.user_name = "sakura2"
        let duplicate = Person(id: "55555555-5555-4555-8555-555555555555", user_name: other.user_name, display_name: other.name)
        var bot = peer; bot.is_bot = true
        var inactive = peer; inactive.is_deactivated = true
        let rows = [user, peer, duplicate, bot, inactive]
        harness.stub { _ in
            var events = try rows.enumerated().map { index, person in
                JSONValue.object(["key": .string(String(index)), "headers": .object(["operation": .string("insert")]),
                                  "value": try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(person))])
            }
            events.append(.object(["headers": .object(["control": .string("up-to-date")])]))
            return (200, ["electric-handle": "users", "electric-offset": "0_1"], try JSONEncoder().encode(events))
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let recipients = try await ShortcutActions(workspace: workspace).recipients()
        #expect(recipients.count == 2)
        #expect(recipients.filter { $0.matches("さくら") }.count == 2)
        #expect(recipients.filter { $0.matches("SAKURA2") }.count == 1)
        #expect(recipients.allSatisfy { !$0.matches(" ") })
    }

    @Test("相手を再検証してDMへ送信し、既存の下書きを保持する")
    func sendsToResolvedDM() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let peer = peer, channel = channel
        let sent = Mutex(false)
        harness.stub { request in
            switch request.url?.path {
            case "/api/users/\(peer.id)": return (200, [:], try JSONEncoder().encode(peer))
            case "/api/dms":
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body["target_user_ids"] == .array([.string(peer.id)]))
                return (200, [:], try Self.envelope(channel))
            case "/api/messages":
                let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
                #expect(body["channel_id"] == .string(channel.id))
                #expect(body["content"] == .string("Siriからのメッセージ"))
                #expect(body["parent_id"] == nil)
                sent.withLock { $0 = true }
                return (200, [:], try Self.envelope(Message(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", user_id: "you",
                    channel_id: channel.id, content: "Siriからのメッセージ", created_at: "2026-11-20T00:00:00Z")))
            default: Issue.record("想定外の通信です"); return (500, [:], Data())
            }
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let conversation = workspace.conversation(for: channel)
        conversation.drafts["root"] = "入力途中の下書き"
        try await ShortcutActions(workspace: workspace).send("Siriからのメッセージ", to: recipient(workspace))
        #expect(sent.withLock { $0 })
        #expect(workspace.selectedTab == .directMessages)
        #expect(workspace.dmPath == [.channel(channel.id)])
        #expect(conversation.drafts["root"] == "入力途中の下書き")
    }

    @Test("別アカウント・無効な宛先・編集不可の会話・空本文には送信しない",
          arguments: ["account", "inactive", "bot", "encrypted", "archived", "deleted", "empty", "long"])
    func preventsInvalidSend(kind: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        var peer = peer, channel = channel
        if kind == "inactive" { peer.is_deactivated = true }
        if kind == "bot" { peer.is_bot = true }
        if kind == "encrypted" { channel.encrypted = true }
        if kind == "archived" { channel.archive_at = "2026-11-20T00:00:00Z" }
        if kind == "deleted" { channel.deleted_at = "2026-11-20T00:00:00Z" }
        let personData = try JSONEncoder().encode(peer), channelData = try Self.envelope(channel)
        harness.stub { request in
            if request.url?.path.hasPrefix("/api/users/") == true { return (200, [:], personData) }
            if request.url?.path == "/api/dms" { return (200, [:], channelData) }
            Issue.record("不正な入力で送信してはいけません")
            return (500, [:], Data())
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        var target = recipient(workspace)
        if kind == "account" { target = .init(id: "別アカウント", personID: target.personID, name: target.name, username: target.username) }
        let content = kind == "empty" ? " \n" : kind == "long" ? String(repeating: "あ", count: 40_001) : "本文"
        await #expect(throws: ZooError.self) { try await ShortcutActions(workspace: workspace).send(content, to: target) }
    }

    @Test("DM作成中にログアウトしたら送信しない")
    func logoutWhileResolvingDM() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let peer = peer, channel = channel
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { request in
            if request.url?.path == "/api/users/\(peer.id)" { return (200, [:], try JSONEncoder().encode(peer)) }
            if request.url?.path == "/api/dms" {
                started.continuation.yield(())
                for await _ in release.stream { break }
                return (200, [:], try Self.envelope(channel))
            }
            Issue.record("ログアウト後に送信してはいけません")
            return (500, [:], Data())
        }
        let workspace = WorkspaceStore(client: harness.client, user: user, recentEmojiDefaults: nil)
        let actions = ShortcutActions(workspace: workspace), target = recipient(workspace)
        let pending = Task { try await actions.send("本文", to: target) }
        for await _ in started.stream { break }
        workspace.shutdown()
        release.continuation.yield(())
        await #expect(throws: ZooError.self) { try await pending.value }
        #expect(workspace.dmPath.isEmpty)
    }
}
