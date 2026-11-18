import Foundation
import Testing
@testable import Zoo

@Suite("通話プロトコル")
struct CallProtocolTests {
    @Test("SSEの空行境界とkeepaliveを処理する")
    func events() throws {
        var parser = CallEventParser()
        #expect(try parser.feed(": keepalive") == nil)
        #expect(try parser.feed("") == nil)
        _ = try parser.feed("event: participant_left")
        _ = try parser.feed("data: {\"session_id\":\"session\"}")
        guard case .left(let id) = try parser.feed("") else { Issue.record("退出イベントがありません"); return }
        #expect(id == "session")
        _ = try parser.feed("event: room_state")
        _ = try parser.feed(#"data: {"participants":[],"max_participants":6,"ice_servers":[{"urls":"stun:stun.example.org"},{"urls":["turn:turn.example.org"],"username":"user","credential":"password"}]}"#)
        guard case .room(let room) = try parser.feed("") else { Issue.record("初期状態がありません"); return }
        #expect(room.ice_servers.count == 2)
        #expect(room.ice_servers[0].urls == ["stun:stun.example.org"])
    }

    @Test("過大なシグナルを送信しない")
    func oversizedSignal() {
        #expect(throws: ZooError.self) { try CallSignal(type: "offer", sdp: String(repeating: "a", count: 6_500)).json() }
    }
}

@MainActor
private final class FakeCallClient: CallClient {
    var events: [CallEvent] = []
    var commands: [String] = []
    var observed = false
    var commandObserved: (@MainActor (String) -> Void)?
    var accepted = false
    var failStream = false
    var delivered: (@MainActor () -> Void)?

    func observeCall(channelID: String, sessionID: String, receive: @MainActor (CallEvent) async throws -> Void) async throws {
        observed = true
        for event in events { try await receive(event) }
        delivered?()
        if failStream { throw URLError(.networkConnectionLost) }
        try await Task.sleep(for: .seconds(86_400))
    }
    func callCommand(channelID: String, action: String, body: [String: JSONValue]) async throws { commands.append(action); commandObserved?(action) }
    func observeCallInvitations(receive: @MainActor ([CallInvitation]) -> Void) async throws { receive([]) }
    func answerCallInvitation(id: String, accept: Bool) async throws { accepted = accept }
}

@MainActor
private final class FakeCallMedia: CallMedia {
    var started = false
    var closed = false
    var muted = false
    var handled: [String] = []
    var participantIDs: [String] = []
    var signal: (@MainActor (String, CallSignal) -> Void)?
    func start(sessionID: String, room: CallRoom, signal: @escaping @MainActor (String, CallSignal) -> Void,
               failed: @escaping @MainActor () -> Void) throws { started = true; self.signal = signal }
    func handle(_ event: CallEvent) async throws {
        switch event { case .room(let room): handled.append("room"); participantIDs = room.participants.map(\.id); case .left: handled.append("left"); default: handled.append("other") }
    }
    func mute(_ muted: Bool) { self.muted = muted }
    func speaker(_ enabled: Bool) throws {}
    func close() { closed = true; signal = nil }
}

@MainActor @Suite("通話のライフサイクル", .timeLimit(.minutes(1)))
struct CallStoreTests {
    private var channel: Channel { Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "一般", is_private: false, is_dm: false) }

    @Test("初期状態より先のイベントを保留し、終了時にマイクと接続を閉じる")
    func lifecycle() async throws {
        let client = FakeCallClient()
        let remote = CallParticipant(session_id: "remote", user_id: "other", user_name: "other", display_name: "相手", is_muted: false)
        client.events = [.left("remote"), .room(CallRoom(participants: [remote], max_participants: 6, ice_servers: []))]
        let media = FakeCallMedia()
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { true })
        await confirmation("初期イベントを受信") { confirmation in
            await withCheckedContinuation { continuation in
                client.delivered = { confirmation(); continuation.resume() }
                store.start(channel: channel)
            }
        }
        #expect(media.handled == ["left", "room", "left"])
        #expect(store.ready)
        #expect(media.participantIDs.isEmpty && store.participants.isEmpty)
        store.toggleMute()
        #expect(media.muted)
        await withCheckedContinuation { continuation in
            client.commandObserved = { if $0 == "leave" { continuation.resume() } }
            store.finish()
        }
        #expect(media.closed)
        #expect(store.channel == nil)
        #expect(!store.ready)
        #expect(client.commands.contains("leave"))
    }

    @Test("マイク拒否時は招待を承認せず入室もしない")
    func deniedMicrophone() async {
        let client = FakeCallClient()
        let media = FakeCallMedia()
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { false })
        await store.start(channel: channel, invitationID: "invite")?.value
        #expect(store.error != nil)
        #expect(!client.observed && !client.accepted && !media.started)
        #expect(store.channel == nil)
    }

    @Test("切断後にメディアを残さず終了し、再入室できる")
    func streamFailure() async {
        let client = FakeCallClient()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        client.failStream = true
        let media = FakeCallMedia()
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { true })
        await store.start(channel: channel)?.value
        #expect(store.error != nil && media.closed)
        #expect(store.channel == nil)
        store.shutdown()
        store.start(channel: channel)
        #expect(store.channel == nil)
    }
}

@MainActor @Suite("通話のネイティブAPI契約")
struct NativeCallAPITests {
    @Test("SSEの改行境界を保ち、認証付きで初期状態と退出を受け取る")
    func stream() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.value(forHTTPHeaderField: "Accept") == "text/event-stream")
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            #expect(request.url?.query?.contains("session_id=") == true)
            let events = "event: room_state\r\ndata: {\"participants\":[],\"max_participants\":6,\"ice_servers\":[]}\r\n\r\n: keepalive\n\nevent: participant_left\ndata: {\"session_id\":\"remote\"}\n\n"
            return (200, ["Content-Type": "text/event-stream"], Data(events.utf8))
        }
        var received: [String] = []
        do {
            try await harness.client.observeCall(channelID: "11111111-1111-4111-8111-111111111111",
                                                 sessionID: "22222222-2222-4222-8222-222222222222") { event in
                switch event {
                case .room: received.append("room")
                case .left(let id): received.append(id)
                default: Issue.record("予期しないイベント")
                }
            }
            Issue.record("SSEの終了を検出していません")
        } catch let error as URLError { #expect(error.code == .networkConnectionLost) }
        #expect(received == ["room", "remote"])
    }

    @Test("通話の書き込みは非Envelope応答を受け付ける")
    func command() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path.hasSuffix("/call/state") == true)
            let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
            #expect(body["is_muted"] == .bool(true))
            return (200, [:], Data(#"{"ok":true}"#.utf8))
        }
        try await harness.client.callCommand(channelID: "11111111-1111-4111-8111-111111111111", action: "state",
                                             body: ["session_id": .string("22222222-2222-4222-8222-222222222222"), "is_muted": .bool(true)])
    }
}
