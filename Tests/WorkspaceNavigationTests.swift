import Foundation
import Testing
@testable import Zoo

@MainActor
@Suite("一覧から前の会話へ戻る")
struct WorkspaceNavigationTests {
    @Test("一覧へ戻った後は最後のチャンネルを開き、下書きとStoreを維持する")
    func reopensMostRecentChannel() async throws {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        await workspace.refresh()
        #expect(workspace.resumableChannel == nil)
        let channel = try #require(workspace.channels.first { $0.channel_name == "一般" })
        workspace.addAndOpen(channel)
        let store = workspace.conversation(for: channel)
        store.drafts["root"] = "保存した下書き"
        #expect(workspace.resumableChannel == nil)
        workspace.channelPath = []
        #expect(workspace.resumableChannel?.id == channel.id)
        workspace.selectedTab = .directMessages
        workspace.reopenLastChannel()
        #expect(workspace.channelPath.isEmpty)
        workspace.selectedTab = .channels
        workspace.reopenLastChannel()
        #expect(workspace.channelPath == [.channel(channel.id)])
        #expect(workspace.conversation(for: channel) === store)
        #expect(store.drafts["root"] == "保存した下書き")
        let next = try #require(workspace.channels.first { $0.channel_name == "開発" })
        workspace.channelPath = [.channel(next.id), .thread(channelID: next.id, parentID: "reply", focusComposer: false)]
        workspace.channelPath = []
        workspace.reopenLastChannel()
        #expect(workspace.channelPath == [.channel(next.id)])
    }

    @Test("未参加でも閲覧できる公開チャンネルを開き直せる")
    func reopensUnjoinedPublicChannel() {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let channel = Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "公開", is_private: false, is_dm: false)
        workspace.addAndOpen(channel, joined: false)
        workspace.channelPath = []
        #expect(!workspace.joinedChannelIDs.contains(channel.id))
        workspace.reopenLastChannel()
        #expect(workspace.channelPath == [.channel(channel.id)])
    }

    @Test("アーカイブ・削除・API一覧からの除外・権限喪失・ログアウト後の会話は開かない", arguments: ["archive", "delete", "removed", "revoke", "logout"])
    func ignoresUnavailableChannel(reason: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            switch request.url?.path {
            case "/api/shapes/users":
                return (200, ["electric-handle": "users", "electric-offset": "0_1"],
                        Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
            case "/api/channels", "/api/channel-read-states": return (200, [:], Data("[]".utf8))
            default: return (403, [:], Data())
            }
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        var channel = Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "一般", is_private: false, is_dm: false)
        workspace.addAndOpen(channel)
        if reason == "archive" { channel.archive_at = "2026-11-20T00:00:00Z"; workspace.addAndOpen(channel) }
        if reason == "delete" { channel.deleted_at = "2026-11-20T00:00:00Z"; workspace.addAndOpen(channel) }
        if reason == "removed" { await workspace.refresh(); #expect(workspace.error == nil) }
        if reason == "revoke" { await workspace.conversation(for: channel).run() }
        if reason == "logout" { workspace.shutdown() }
        workspace.channelPath = []
        workspace.reopenLastChannel()
        #expect(workspace.resumableChannel == nil)
        #expect(workspace.channelPath.isEmpty)
    }
}
