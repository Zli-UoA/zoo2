import AVKit
import Synchronization
import Testing
@testable import Zoo

@MainActor @Suite("添付動画の再生状態", .timeLimit(.minutes(1)))
struct AttachmentVideoTests {
    @Test("センシティブな動画は開くまで先頭フレームを取得せず、取得後は保持する")
    func posterPrivacyAndCache() async throws {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let attachment = MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa02", file_name: "動画.mp4",
                                           content_type: "video/mp4", is_sensitive: true)
        let state = AttachmentRenderState(attachment: attachment)
        await state.load(workspace: workspace, channelID: "11111111-1111-4111-8111-111111111111")
        #expect(state.url == nil && state.videoThumbnail == nil)
        state.revealed = true
        await state.load(workspace: workspace, channelID: "11111111-1111-4111-8111-111111111111")
        let poster = try #require(state.videoThumbnail)
        #expect(poster.size.width == 320 && poster.size.height == 180)
        state.cancelLoad()
        await state.load(workspace: workspace, channelID: "11111111-1111-4111-8111-111111111111")
        #expect(state.videoThumbnail === poster)
        #expect(workspace.videoPlayback.session == nil)
    }

    @Test("標準操作による一時停止で音声を解放し、再開時に再取得する")
    func pauseReleasesAudioSession() async throws {
        var activations = 0
        var deactivations = 0
        var activated: CheckedContinuation<Void, Never>?
        var deactivated: CheckedContinuation<Void, Never>?
        let session = AttachmentVideoSession(attachmentID: "video", channelID: "channel", activateAudio: {
            activations += 1
            activated?.resume(); activated = nil
        }, deactivateAudio: {
            deactivations += 1
            deactivated?.resume(); deactivated = nil
        }, restore: { false })
        let url = try #require(Bundle.main.url(forResource: "PreviewVideo", withExtension: "mp4"))
        try session.prepare(url: url)
        await withCheckedContinuation { continuation in
            deactivated = continuation
            session.player.pause()
        }
        #expect(activations == 1 && deactivations == 1)
        await withCheckedContinuation { continuation in
            activated = continuation
            session.player.play()
        }
        #expect(activations == 2)
        session.stop()
        #expect(deactivations == 2)
    }

    @Test("行が消えてもPiPのプレーヤーと再生対象を保持し、終了時に解放する")
    func pictureInPictureLifetime() throws {
        let session = AttachmentVideoSession(attachmentID: "video", channelID: "channel", activateAudio: {}, deactivateAudio: {}) { false }
        let url = try #require(Bundle.main.url(forResource: "PreviewVideo", withExtension: "mp4"))
        try session.prepare(url: url)
        let item = try #require(session.player.currentItem)
        session.playerViewControllerWillStartPictureInPicture(session.controller)
        session.setInlineVisible(false)
        #expect(session.pictureInPictureActive)
        #expect(session.controller.player === session.player)
        #expect(session.player.currentItem === item)
        session.playerViewControllerDidStopPictureInPicture(session.controller)
        #expect(session.player.rate == 0)
        session.stop()
        #expect(session.player.currentItem == nil)
        #expect(session.controller.player == nil)
    }

    @Test("復帰先を表示できなかった場合も完了通知を一度だけ返す")
    func failedRestoration() async {
        let session = AttachmentVideoSession(attachmentID: "video", channelID: "channel", activateAudio: {}, deactivateAudio: {}) { false }
        session.setInlineVisible(false)
        var completions: [Bool] = []
        await withCheckedContinuation { continuation in
            session.playerViewController(session.controller, restoreUserInterfaceForPictureInPictureStopWithCompletionHandler: {
                completions.append($0)
                continuation.resume()
            })
        }
        session.stop()
        #expect(completions == [false])
    }

    @Test("別の動画への切り替えと停止で、開始前のURL取得をキャンセルする")
    func cancelledLoads() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in
            Issue.record("キャンセルした再生から通信が始まりました")
            return (500, [:], Data())
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let playback = workspace.videoPlayback
        playback.play(MessageAttachment(id: "first", file_name: "動画1.mp4", content_type: "video/mp4"),
                      workspace: workspace, channelID: "channel", messageID: "message")
        let first = try #require(playback.session)
        let firstTask = first.loadTask
        playback.play(MessageAttachment(id: "second", file_name: "動画2.mp4", content_type: "video/mp4"),
                      workspace: workspace, channelID: "channel", messageID: "message")
        let second = try #require(playback.session)
        let secondTask = second.loadTask
        #expect(first !== second)
        playback.stop()
        await firstTask?.value
        await secondTask?.value
        #expect(playback.session == nil)
        #expect(first.player.currentItem == nil && second.player.currentItem == nil)
    }

    @Test("読み込み失敗後の再試行では、署名URLを取得し直す")
    func retryRefreshesURL() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let requests = Mutex(0)
        harness.stub { _ in
            requests.withLock { $0 += 1 }
            return (500, [:], Data())
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let attachment = MessageAttachment(id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa02", file_name: "動画.mp4", content_type: "video/mp4")
        workspace.videoPlayback.play(attachment, workspace: workspace, channelID: "channel", messageID: "message")
        let first = try #require(workspace.videoPlayback.session)
        await first.loadTask?.value
        #expect(first.error != nil)
        workspace.videoPlayback.play(attachment, workspace: workspace, channelID: "channel", messageID: "message")
        let second = try #require(workspace.videoPlayback.session)
        #expect(second !== first && second.error == nil)
        await second.loadTask?.value
        #expect(second.error != nil)
        #expect(requests.withLock { $0 } == 2)
    }

    @Test("画面移動ではPiPを保持し、閲覧権限の失効・会話の削除・暗号化では停止する", arguments: ["403", "404", "removed", "encrypted"])
    func accessLossStopsPictureInPicture(reason: String) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            switch request.url?.path {
            case "/api/shapes/messages":
                return (Int(reason) ?? 500, [:], Data())
            case "/api/shapes/users":
                return (200, ["electric-handle": "users", "electric-offset": "0_1"],
                        Data(#"[{"headers":{"control":"up-to-date"}}]"#.utf8))
            case "/api/channels", "/api/channel-read-states":
                return (200, [:], Data("[]".utf8))
            default:
                Issue.record("想定外の通信が発生しました")
                return (500, [:], Data())
            }
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        var channel = Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "動画確認", is_private: true, is_dm: false)
        let store = workspace.conversation(for: channel)
        workspace.channelPath = [.channel(channel.id)]
        workspace.videoPlayback.play(MessageAttachment(id: "video", file_name: "動画.mp4", content_type: "video/mp4"),
                                     workspace: workspace, channelID: channel.id, messageID: "message")
        let session = try #require(workspace.videoPlayback.session)
        session.loadTask?.cancel()
        await session.loadTask?.value
        let url = try #require(Bundle.main.url(forResource: "PreviewVideo", withExtension: "mp4"))
        let item = AVPlayerItem(url: url)
        session.player.replaceCurrentItem(with: item)
        session.playerViewControllerWillStartPictureInPicture(session.controller)
        session.setInlineVisible(false)
        workspace.channelPath = []
        workspace.selectedTab = .profile
        let other = Channel(id: "other", channel_name: "別の会話", is_private: false, is_dm: false)
        workspace.conversation(for: other).shutdown()
        #expect(workspace.videoPlayback.session === session)
        #expect(session.player.currentItem === item)

        switch reason {
        case "removed":
            await workspace.refresh()
            #expect(workspace.error == nil)
        case "encrypted":
            channel.encrypted = true
            store.update(channel)
        default:
            await store.run()
            #expect(store.blocked)
        }
        #expect(workspace.videoPlayback.session == nil)
        #expect(session.player.currentItem == nil)
        #expect(session.controller.player == nil)
    }

    @Test("通話開始時に動画を停止し、通話中の再生要求を受け付けない")
    func callTakesAudioSession() throws {
        let client = PreviewChatClient()
        let calls = CallStore(client: client, userID: client.currentUser.id, permission: { false })
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil, calls: calls)
        defer { workspace.shutdown() }
        let attachment = MessageAttachment(id: "video", file_name: "動画.mp4", content_type: "video/mp4")
        workspace.videoPlayback.play(attachment, workspace: workspace, channelID: "channel", messageID: "message")
        #expect(workspace.videoPlayback.session != nil)
        let channel = Channel(id: "channel", channel_name: "動画確認", is_private: false, is_dm: false)
        calls.start(channel: channel)
        #expect(workspace.videoPlayback.session == nil)
        workspace.videoPlayback.play(attachment, workspace: workspace, channelID: "channel", messageID: "message")
        #expect(workspace.videoPlayback.session == nil)
    }
}
