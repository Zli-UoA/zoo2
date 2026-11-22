import AppIntents
import Testing
@testable import Zoo

@MainActor
@Suite("Siriの通話相手と発信", .timeLimit(.minutes(1)))
struct SiriCallTests {
    @available(iOS 27.0, *)
    @Test("通話スキーマは現在の1人だけを解決し、複数宛先・別アカウント・ビデオを拒否する")
    func schemaDestination() throws {
        let recipient = ShortcutRecipient(id: "server|account|person", personID: "person", name: "メンバー", username: "member")
        let person = ZooPhonePerson(recipient)
        #expect(try ZooCallDestination.person(person).resolve(in: [recipient]) == recipient)
        #expect(try ZooCallDestination.people([person]).resolve(in: [recipient]) == recipient)
        #expect(throws: ZooError.self) { try ZooCallDestination.people([person, person]).resolve(in: [recipient]) }
        #expect(throws: ZooError.self) { try ZooCallDestination.people([]).resolve(in: [recipient]) }
        #expect(throws: ZooError.self) { try ZooCallDestination.person(person).resolve(in: []) }
        try ZooCallAudioVisualMode.audio.validate()
        #expect(throws: ZooError.self) { try ZooCallAudioVisualMode.video.validate() }
    }

    @Test("Siriの通話表示を音声より先に開始し、終了・失敗・表示の取り消しで片付ける",
          arguments: ["success", "activityFailure", "systemFailure", "denied", "dismissed", "ordinary"])
    func liveActivityLifecycle(kind: String) async throws {
        let client = FakeCallClient()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let activity = FakeCallLiveActivity()
        activity.reject = kind == "activityFailure"
        let media = FakeCallMedia()
        let calls = CallStore(client: client, userID: "自分", makeMedia: { media }, permission: { kind != "denied" },
                              makeLiveActivity: { activity })
        defer { calls.shutdown() }
        var preparedSystemCall = false
        calls.prepareSystemCall = { _, _ in
            #expect(activity.started == (kind != "ordinary"))
            preparedSystemCall = true
            if kind == "systemFailure" { throw ZooError.invalidResponse }
        }
        let channel = Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "通話", is_private: false, is_dm: true)
        if ["activityFailure", "systemFailure", "denied"].contains(kind) {
            await #expect(throws: ZooError.self) {
                try await calls.startAndWait(channel: channel, requiresLiveActivity: true)
            }
            #expect(!media.started && !client.observed && calls.channel == nil)
            #expect(preparedSystemCall == (kind == "systemFailure"))
            #expect(activity.finished == (kind == "denied" ? 0 : 1))
        } else {
            try await calls.startAndWait(channel: channel, requiresLiveActivity: kind != "ordinary")
            #expect(media.started && calls.ready)
            if kind == "dismissed" { activity.onEnded?() } else { calls.finish() }
            #expect(media.closed && calls.channel == nil)
            #expect(activity.finished == (kind == "ordinary" ? 0 : 1))
        }
    }

    @Test("Siri発信はZooのDMに接続し、既存通話・権限拒否・古い宛先では発信しない",
          arguments: ["success", "denied", "stale", "signedOut"])
    func startsZooCall(kind: String) async throws {
        let client = PreviewChatClient()
        let callClient = FakeCallClient()
        callClient.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let calls = CallStore(client: callClient, userID: client.currentUser.id,
                              makeMedia: { PreviewCallMedia() }, permission: { kind != "denied" })
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil, calls: calls)
        defer { workspace.shutdown() }
        let actions = ShortcutActions(workspace: workspace)
        var target = try #require(try await actions.recipients().first)
        if kind == "stale" {
            target = ShortcutRecipient(id: "別アカウント", personID: target.personID, name: target.name, username: target.username)
        }
        if kind == "signedOut" { workspace.shutdown() }
        if kind == "success" {
            try await actions.call(target)
            let channel = try #require(calls.channel)
            #expect(channel.is_dm && calls.ready)
            await #expect(throws: ZooError.self) { try await actions.call(target) }
            #expect(calls.channel?.id == channel.id && calls.ready)
        } else {
            await #expect(throws: (any Error).self) { try await actions.call(target) }
            #expect(calls.channel == nil && !calls.ready)
        }
    }
}

@MainActor
private final class FakeCallLiveActivity: CallLiveActivity {
    var reject = false
    var started = false
    var finished = 0
    var onEnded: (@MainActor () -> Void)?

    func start(onEnded: @escaping @MainActor () -> Void) throws {
        if reject { throw ZooError.invalidResponse }
        started = true
        self.onEnded = onEnded
    }

    func finish() { finished += 1; onEnded = nil }
}
