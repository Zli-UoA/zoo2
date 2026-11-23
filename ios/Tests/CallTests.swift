import Foundation
import Testing
@testable import Zoo

@MainActor
func waitForCallDelivery(_ client: FakeCallClient, start: () -> Task<Void, Never>?) async throws {
    let delivered = AsyncStream<Void>.makeStream()
    client.delivered = { delivered.continuation.yield(()) }
    defer { client.delivered = nil; delivered.continuation.finish() }
    let connection = try #require(start(), "入室処理を開始できませんでした。")
    let completion = Task {
        await connection.value
        delivered.continuation.finish()
    }
    defer { completion.cancel() }
    var iterator = delivered.stream.makeAsyncIterator()
    let received = await iterator.next()
    try Task.checkCancellation()
    try #require(received != nil, "ルームを受信する前に接続処理が終了しました。")
}

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
        try #require(room.ice_servers.count == 2)
        #expect(room.ice_servers[0].urls == ["stun:stun.example.org"])
    }

    @Test("過大なシグナルを送信しない")
    func oversizedSignal() {
        #expect(throws: ZooError.self) { try CallSignal(type: "offer", sdp: String(repeating: "a", count: 6_500)).json() }
    }
}

@MainActor
final class FakeCallClient: CallClient {
    var activeCallRows: [ActiveCallSummary] = []
    var activeCallsFailure: Error?
    var beforeActiveCallsResult: (@MainActor () async -> Void)?
    var events: [CallEvent] = []
    var commands: [String] = []
    var observed = false
    var sessions: [String] = []
    var commandFailure: (@MainActor (String) -> Error?)?
    var commandBodies: [(String, [String: JSONValue])] = []
    var commandObserved: (@MainActor (String) -> Void)?
    var beforeCommand: (@MainActor (String) async throws -> Void)?
    var commandChannels: [String] = []
    var accepted = false
    var answeredInvitations: [(String, Bool)] = []
    var failStream = false
    var delivered: (@MainActor () -> Void)?
    var beforeObserveCall: (@MainActor (String, String) -> Void)?
    var inviteRequests: [(String, [String])] = []
    var cancelledInvitations: [String] = []
    var inviteResult: [CreatedCallInvitation] = []
    var inviteFailure: Error?
    var beforeInviteResult: (@MainActor () async -> Void)?
    var beforeCancelResult: (@MainActor () async -> Void)?
    var invitationRows: [CallInvitation] = []
    var invitationsDelivered: (@MainActor () -> Void)?
    let invitationUpdates = AsyncStream<[CallInvitation]>.makeStream()

    func activeCalls() async throws -> [ActiveCallSummary] {
        let rows = activeCallRows
        await beforeActiveCallsResult?()
        if let activeCallsFailure { throw activeCallsFailure }
        return rows
    }
    func observeCall(channelID: String, sessionID: String, receive: @MainActor (CallEvent) async throws -> Void) async throws {
        beforeObserveCall?(channelID, sessionID)
        observed = true
        sessions.append(sessionID)
        for event in events { try await receive(event) }
        delivered?()
        if failStream { throw URLError(.networkConnectionLost) }
        try await Task.sleep(for: .seconds(86_400))
    }
    func callCommand(channelID: String, action: String, body: [String: JSONValue]) async throws {
        commands.append(action); commandBodies.append((action, body))
        commandChannels.append(channelID)
        try await beforeCommand?(action)
        let failure = commandFailure?(action)
        commandObserved?(action)
        if let failure { throw failure }
    }
    func observeCallInvitations(receive: @MainActor ([CallInvitation]) -> Void) async throws {
        receive(invitationRows)
        invitationsDelivered?()
        for await rows in invitationUpdates.stream {
            receive(rows)
            invitationsDelivered?()
        }
    }
    func answerCallInvitation(id: String, accept: Bool) async throws {
        accepted = accept
        answeredInvitations.append((id, accept))
    }
    func isCallInvitationPending(id: String) async throws -> Bool { true }
    func inviteToCall(channelID: String, userIDs: [String]) async throws -> [CreatedCallInvitation] {
        inviteRequests.append((channelID, userIDs))
        await beforeInviteResult?()
        if let inviteFailure { throw inviteFailure }
        return inviteResult
    }
    func cancelCallInvitation(id: String) async throws {
        await beforeCancelResult?()
        if let inviteFailure { throw inviteFailure }
        cancelledInvitations.append(id)
    }
}

@MainActor
final class FakeCallMedia: CallMedia {
    var onPeerState: (@MainActor (String, CallPeerState) -> Void)?
    var started = false
    var closed = false
    var muted = false
    var handled: [String] = []
    var participantIDs: [String] = []
    var mutedWhenHandlingRoom: Bool?
    var beforeHandle: (@MainActor (CallEvent) async throws -> Void)?
    var signal: (@MainActor (String, CallSignal) -> Void)?
    func start(sessionID: String, room: CallRoom, signal: @escaping @MainActor (String, CallSignal) -> Void,
               failed: @escaping @MainActor () -> Void) throws { started = true; self.signal = signal }
    func handle(_ event: CallEvent) async throws {
        try await beforeHandle?(event)
        if case .room = event { mutedWhenHandlingRoom = muted }
        switch event { case .room(let room): handled.append("room"); participantIDs = room.participants.map(\.id); case .left: handled.append("left"); default: handled.append("other") }
    }
    func mute(_ muted: Bool) { self.muted = muted }
    func speaker(_ enabled: Bool) throws {}
    func diagnostics() async -> [CallPeerDiagnostics] { [] }
    func close() { closed = true; signal = nil }
}

@MainActor @Suite("通話のライフサイクル", .timeLimit(.minutes(1)))
struct CallStoreTests {
    private var channel: Channel { Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "一般", is_private: false, is_dm: false) }

    @Test("初期メディアの処理中に生成したofferを既存参加者へ送信する")
    func initialOfferReachesExistingParticipant() async throws {
        let client = FakeCallClient(), media = FakeCallMedia()
        let peer = CallParticipant(session_id: "peer", user_id: "other", user_name: "相手", display_name: "相手", is_muted: false)
        client.events = [.room(CallRoom(participants: [peer], max_participants: 6, ice_servers: []))]
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { true })
        defer { media.beforeHandle = nil; store.shutdown() }
        media.beforeHandle = { event in
            guard case .room = event else { return }
            #expect(!store.ready)
            media.signal?(peer.id, CallSignal(type: "offer", sdp: "テスト用SDP"))
            let deadline = ContinuousClock.now.advanced(by: .seconds(2))
            while !client.commands.contains("signal"), ContinuousClock.now < deadline {
                try await Task.sleep(for: .milliseconds(1))
            }
            try #require(client.commands.contains("signal"), "メディア初期化の完了前でもofferを送信する")
            #expect(!store.ready)
        }
        try await waitForCallDelivery(client) { store.start(channel: channel) }
        let sent = try #require(client.commandBodies.first { $0.0 == "signal" })
        #expect(sent.1["target_session_id"] == .string(peer.id))
        #expect(store.ready && store.participants.map(\.id) == [peer.id])
    }

    @Test("初期メディアの待機中は準備完了にせず、終了した処理で次の通話を上書きしない", arguments: [false, true])
    func initialMediaBelongsToSession(earlyLeave: Bool) async throws {
        let client = FakeCallClient()
        let firstMedia = FakeCallMedia(), secondMedia = FakeCallMedia()
        var engines = [firstMedia, secondMedia]
        var firstGate: CheckedContinuation<Void, Never>?, secondGate: CheckedContinuation<Void, Never>?
        firstMedia.beforeHandle = { event in
            let isLeft: Bool
            if case .left = event { isLeft = true } else { isLeft = false }
            if isLeft == earlyLeave { await withCheckedContinuation { firstGate = $0 } }
        }
        secondMedia.beforeHandle = { _ in await withCheckedContinuation { secondGate = $0 } }
        let store = CallStore(client: client, userID: "me", makeMedia: { engines.removeFirst() }, permission: { true })
        defer { store.shutdown(); firstGate?.resume(); secondGate?.resume() }
        let room = CallRoom(participants: [], max_participants: 6, ice_servers: [])
        client.events = earlyLeave ? [.left("退出済み"), .room(room)] : [.room(room)]
        let first = try #require(store.start(channel: channel))
        var deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while firstGate == nil, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
        try #require(firstGate != nil)
        #expect(!store.ready)
        store.finish()
        client.events = [.room(room)]
        let second = try #require(store.start(channel: channel))
        defer { second.cancel() }
        deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while secondGate == nil, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
        try #require(secondGate != nil)
        let old = firstGate; firstGate = nil; old?.resume()
        await first.value
        #expect(!store.ready && store.participants.isEmpty)
        let current = secondGate; secondGate = nil; current?.resume()
        deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while !store.ready, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
        #expect(store.ready)
    }

    @Test("入室と音声接続を区別し、一部の相手だけ未接続でも通話中にしない")
    func mediaConnectionStatus() async throws {
        let client = FakeCallClient()
        let media = FakeCallMedia()
        let peers = ["one", "two"].map {
            CallParticipant(session_id: $0, user_id: $0, user_name: $0, display_name: $0, is_muted: false)
        }
        client.events = [.room(CallRoom(participants: peers, max_participants: 6, ice_servers: []))]
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { true })
        defer { store.shutdown() }
        await withCheckedContinuation { done in client.delivered = { done.resume() }; store.start(channel: channel) }
        #expect(store.ready && store.connectionStatus == "音声を接続中")
        media.onPeerState?("one", .connected)
        #expect(store.connectionStatus == "音声を接続中")
        media.onPeerState?("two", .connected)
        #expect(store.connectionStatus == "通話中")
        media.onPeerState?("one", .reconnecting)
        #expect(store.connectionStatus == "音声を再接続中")
        let stale = media.onPeerState
        store.finish()
        stale?("one", .connected)
        #expect(store.peerStates.isEmpty)
    }

    @Test("heartbeatの一時失敗を再試行し、回復した通話を終了しない")
    func heartbeatRecovers() async throws {
        let client = FakeCallClient()
        let media = FakeCallMedia()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        var attempts = 0
        let completed = AsyncStream<Void>.makeStream()
        client.commandFailure = { action in
            guard action == "heartbeat" else { return nil }
            attempts += 1
            return attempts < 3 ? URLError(.networkConnectionLost) : nil
        }
        client.commandObserved = { if $0 == "heartbeat", attempts == 3 { completed.continuation.yield(()) } }
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { true }, retryDelay: .milliseconds(1))
        defer { completed.continuation.finish(); store.shutdown() }
        store.start(channel: channel)
        for await _ in completed.stream { break }
        #expect(attempts == 3 && store.ready && store.error == nil)
        #expect(!store.heartbeatRetrying && !media.closed)
    }

    @Test("heartbeatの継続障害は3回で終了し、認証・権限・session消失は再試行しない", arguments: [0, 401, 403, 404])
    func heartbeatFailureIsBounded(status: Int) async throws {
        let client = FakeCallClient()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        client.commandFailure = { action in
            guard action == "heartbeat" else { return nil }
            return status == 0 ? URLError(.timedOut) : ZooError.rejected(status, "テスト")
        }
        let store = CallStore(client: client, userID: "me", makeMedia: { FakeCallMedia() }, permission: { true }, retryDelay: .milliseconds(1))
        defer { store.shutdown() }
        await store.start(channel: channel)?.value
        #expect(client.commands.filter { $0 == "heartbeat" }.count == (status == 0 ? 3 : 1))
        #expect(store.channel == nil && store.error != nil)
        #expect((store.retryChannel != nil) == (status == 0))
    }

    @Test("再試行待ちで終了した通話を復活させない")
    func cancelHeartbeatRetry() async throws {
        let client = FakeCallClient()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let failed = AsyncStream<Void>.makeStream()
        client.commandFailure = { action in action == "heartbeat" ? URLError(.timedOut) : nil }
        client.commandObserved = { if $0 == "heartbeat" { failed.continuation.yield(()) } }
        let store = CallStore(client: client, userID: "me", makeMedia: { FakeCallMedia() }, permission: { true })
        defer { failed.continuation.finish(); store.shutdown() }
        let task = store.start(channel: channel)
        for await _ in failed.stream { break }
        store.finish()
        await task?.value
        #expect(client.commands.filter { $0 == "heartbeat" }.count == 1)
        #expect(store.channel == nil && store.error == nil && store.retryChannel == nil)
    }

    @Test("SSE切断では自動再着信せず、手動再接続で新しい参加IDを使う")
    func retryUsesNewSession() async throws {
        let client = FakeCallClient()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        client.failStream = true
        let store = CallStore(client: client, userID: "me", makeMedia: { FakeCallMedia() }, permission: { true })
        defer { store.shutdown() }
        await store.start(channel: channel)?.value
        #expect(client.sessions.count == 1 && store.retryChannel?.id == channel.id)
        client.failStream = false
        await withCheckedContinuation { done in client.delivered = { done.resume() }; store.retry() }
        #expect(client.sessions.count == 2 && Set(client.sessions).count == 2)
        #expect(store.ready && store.error == nil && store.retryChannel == nil)
        #expect(store.connectionStatus == "参加者を待機中")
    }

    @Test("ミュート中の切断後は音声接続前にミュートを戻し、サーバーにも同期する")
    func retryPreservesMute() async throws {
        let client = FakeCallClient()
        let media = FakeCallMedia()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        client.failStream = true
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { true })
        defer { store.shutdown() }
        client.delivered = { store.toggleMute() }
        await store.start(channel: channel)?.value
        #expect(store.retryChannel != nil)
        client.failStream = false
        await withCheckedContinuation { done in client.delivered = { done.resume() }; store.retry() }
        #expect(store.ready && store.muted && media.mutedWhenHandlingRoom == true)
        #expect(client.commandBodies.contains { $0.0 == "state" && $0.1["is_muted"] == .bool(true) && $0.1["session_id"] == .string(client.sessions.last!) })
    }

    @Test("ミュート更新の一時失敗を同じ値で再試行し、通話を継続する")
    func muteUpdateRecovers() async throws {
        let client = FakeCallClient()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let store = CallStore(client: client, userID: "me", makeMedia: { FakeCallMedia() }, permission: { true }, retryDelay: .milliseconds(1))
        defer { store.shutdown() }
        await withCheckedContinuation { done in client.delivered = { done.resume() }; store.start(channel: channel) }
        let updated = AsyncStream<Void>.makeStream()
        defer { updated.continuation.finish() }
        var attempts = 0
        client.commandFailure = { action in
            guard action == "state" else { return nil }
            attempts += 1
            return attempts < 3 ? URLError(.timedOut) : nil
        }
        client.commandObserved = { if $0 == "state", attempts == 3 { updated.continuation.yield(()) } }
        store.toggleMute()
        for await _ in updated.stream { break }
        #expect(store.ready && store.muted && !store.changingMute && store.error == nil)
        #expect(client.commandBodies.filter { $0.0 == "state" }.allSatisfy { $0.1["is_muted"] == .bool(true) })
    }

    @Test("未参加の通話も表示し、参加者ゼロと終了した通話は表示しない")
    func activeCallsFollowServer() async throws {
        let client = FakeCallClient()
        let store = CallStore(client: client, userID: "me")
        defer { store.shutdown() }
        client.activeCallRows = [ActiveCallSummary(channel_id: channel.id, participant_count: 2),
                                 ActiveCallSummary(channel_id: "empty", participant_count: 0)]
        await store.refreshActiveCalls()
        #expect(store.channel == nil)
        #expect(store.hasActiveCall(in: channel.id))
        #expect(!store.hasActiveCall(in: "empty"))
        #expect(!store.hasActiveCall(in: "other"))
        client.activeCallRows = []
        await store.refreshActiveCalls()
        #expect(!store.hasActiveCall(in: channel.id))
    }

    @Test("通話一覧の失敗で古い表示を消し、次の取得で復帰する")
    func activeCallsRecoverAfterFailure() async throws {
        let client = FakeCallClient()
        let store = CallStore(client: client, userID: "me")
        defer { store.shutdown() }
        client.activeCallRows = [ActiveCallSummary(channel_id: channel.id, participant_count: 1)]
        await store.refreshActiveCalls()
        client.activeCallsFailure = URLError(.notConnectedToInternet)
        await store.refreshActiveCalls()
        #expect(store.activeChannelIDs.isEmpty)
        #expect(store.error == nil)
        client.activeCallsFailure = nil
        await store.refreshActiveCalls()
        #expect(store.hasActiveCall(in: channel.id))
    }

    @Test("通話一覧の認証切れをセッションへ通知する")
    func activeCallsReportExpiredSession() async throws {
        let client = FakeCallClient()
        let store = CallStore(client: client, userID: "me")
        defer { store.shutdown() }
        client.activeCallsFailure = ZooError.expiredSession
        await confirmation("認証切れを通知") { expired in
            store.onExpiredSession = { expired() }
            await store.refreshActiveCalls()
        }
    }

    @Test("古い通話一覧の応答で最新状態を巻き戻さない")
    func staleActiveCallsAreIgnored() async throws {
        let client = FakeCallClient()
        let store = CallStore(client: client, userID: "me")
        defer { store.shutdown() }
        client.activeCallRows = [ActiveCallSummary(channel_id: channel.id, participant_count: 1)]
        client.beforeActiveCallsResult = {
            client.beforeActiveCallsResult = nil
            client.activeCallRows = []
            await store.refreshActiveCalls()
        }
        await store.refreshActiveCalls()
        #expect(store.activeChannelIDs.isEmpty)
    }

    @Test("キャンセル・ログアウト後の通話一覧の応答を反映しない", arguments: [false, true])
    func lateActiveCallsAreIgnored(shutdown: Bool) async throws {
        let client = FakeCallClient()
        let store = CallStore(client: client, userID: "me")
        defer { client.beforeActiveCallsResult = nil; store.shutdown() }
        client.activeCallRows = [ActiveCallSummary(channel_id: channel.id, participant_count: 1)]
        client.beforeActiveCallsResult = {
            if shutdown { store.shutdown() }
            else { withUnsafeCurrentTask { $0?.cancel() } }
        }
        await Task { await store.refreshActiveCalls() }.value
        #expect(store.activeChannelIDs.isEmpty)
    }

    @Test("参加した通話は一覧APIの反映を待たず表示し、退出すると消える")
    func localCallAppearsImmediately() async throws {
        let client = FakeCallClient()
        let store = try await connected(client)
        defer { store.shutdown() }
        #expect(store.activeChannelIDs.isEmpty)
        #expect(store.hasActiveCall(in: channel.id))
        store.finish()
        #expect(!store.hasActiveCall(in: channel.id))
    }

    private func connected(_ client: FakeCallClient, channel: Channel? = nil, participants: [CallParticipant] = [], capacity: Int = 6) async throws -> CallStore {
        client.events = [.room(CallRoom(participants: participants, max_participants: capacity, ice_servers: []))]
        let store = CallStore(client: client, userID: "me", makeMedia: { FakeCallMedia() }, permission: { true })
        do { try await waitForCallDelivery(client) { store.start(channel: channel ?? self.channel) } }
        catch { store.shutdown(); throw error }
        return store
    }

    private func observeInvitations(_ client: FakeCallClient, store: CallStore) async -> Task<Void, Never> {
        var task: Task<Void, Never>!
        await withCheckedContinuation { continuation in
            client.invitationsDelivered = { continuation.resume() }
            task = Task { await store.runInvitations() }
        }
        client.invitationsDelivered = nil
        return task
    }

    private func deliverInvitations(_ rows: [CallInvitation], client: FakeCallClient) async {
        await withCheckedContinuation { continuation in
            client.invitationsDelivered = { continuation.resume() }
            client.invitationUpdates.continuation.yield(rows)
        }
        client.invitationsDelivered = nil
    }

    @Test("チャンネルの招待候補から自分・bot・無効な人・参加者・呼び出し中の人を除く")
    func invitationCandidates() async throws {
        let client = FakeCallClient()
        let participant = CallParticipant(session_id: "remote", user_id: "joined", user_name: "joined", display_name: "参加者", is_muted: false)
        let store = try await connected(client, participants: [participant], capacity: 3)
        defer { store.shutdown() }
        let now = Date.now
        func invitation(_ id: String, invitee: String, channelID: String? = nil, expiry: Date? = nil) -> CallInvitation {
            CallInvitation(id: id, channel_id: channelID ?? channel.id, inviter_id: "me", invitee_id: invitee,
                           status: "pending", expires_at: (expiry ?? now.addingTimeInterval(30)).ISO8601Format())
        }
        client.invitationRows = [invitation("pending", invitee: "pending"), invitation("joined", invitee: "joined"),
                                 invitation("expired", invitee: "expired", expiry: now.addingTimeInterval(-1)),
                                 invitation("elsewhere", invitee: "elsewhere", channelID: "other")]
        let task = await observeInvitations(client, store: store)
        defer { task.cancel() }
        var members = ["me", "joined", "pending", "expired", "elsewhere"].map { Person(id: $0, user_name: $0, display_name: $0) }
        members += [Person(id: "bot", user_name: "bot", display_name: "bot", is_bot: true),
                    Person(id: "inactive", user_name: "inactive", display_name: "無効", is_deactivated: true)]
        #expect(store.invitationCandidates(members: members, at: now).map(\.id) == ["expired", "elsewhere"])
        #expect(store.invitationSlots(at: now) == 1)
        #expect(store.invitationSlots(at: now.addingTimeInterval(31)) == 2)
        #expect(store.invitations.isEmpty)
        #expect(!(await store.invite(userIDs: ["expired", "elsewhere"], members: members, channelID: channel.id)))
        #expect(!(await store.invite(userIDs: ["outsider"], members: members, channelID: channel.id)))
        #expect(client.inviteRequests.isEmpty)
    }

    @Test("招待失敗でも通話を続け、選択した相手へ再送できる")
    func retryInvitation() async throws {
        let client = FakeCallClient()
        let store = try await connected(client)
        defer { store.shutdown() }
        let members = [Person(id: "other", user_name: "other", display_name: "相手")]
        client.inviteFailure = URLError(.notConnectedToInternet)
        #expect(!(await store.invite(userIDs: ["other"], members: members, channelID: channel.id)))
        #expect(store.ready && store.channel != nil && store.error == nil)
        #expect(store.invitationError != nil && !store.inviting)
        client.inviteFailure = nil
        client.inviteResult = [CreatedCallInvitation(id: "invite", invitee_id: "other")]
        #expect(await store.invite(userIDs: ["other"], members: members, channelID: channel.id))
        #expect(client.inviteRequests.count == 2)
        #expect(client.inviteRequests.last?.1 == ["other"])
        #expect(store.invitationError == nil && store.invitationNotice == "1人に招待を送りました。")
        #expect(store.invitationCandidates(members: members).isEmpty)
        #expect(store.invitationSlots() == 5)
        if let invitation = store.pendingOutgoing().first {
            await store.cancelInvitation(invitation)
            #expect(client.cancelledInvitations == ["invite"] && store.pendingOutgoing().isEmpty)
        } else { Issue.record("同期前に取り消せる招待がありません") }
    }

    @Test("API応答より先に届いた招待を重複させず、終了済みなら復活させない", arguments: [false, true])
    func inviteSynchronization(endsBeforeResponse: Bool) async throws {
        let client = FakeCallClient()
        defer { client.beforeInviteResult = nil }
        let store = try await connected(client)
        defer { store.shutdown() }
        let task = await observeInvitations(client, store: store)
        defer { task.cancel() }
        let invitation = CallInvitation(id: "invite", channel_id: channel.id, inviter_id: "me", invitee_id: "other",
                                        status: "pending", expires_at: Date.now.addingTimeInterval(30).ISO8601Format())
        client.inviteResult = [CreatedCallInvitation(id: "invite", invitee_id: "other")]
        client.beforeInviteResult = {
            await deliverInvitations([invitation], client: client)
            if endsBeforeResponse { await deliverInvitations([], client: client) }
        }
        let members = [Person(id: "other", user_name: "other", display_name: "相手")]
        #expect(await store.invite(userIDs: ["other"], members: members, channelID: channel.id))
        #expect(store.pendingOutgoing().count == (endsBeforeResponse ? 0 : 1))
        await deliverInvitations([], client: client)
        #expect(store.pendingOutgoing().isEmpty)
    }

    @Test("同期がAPI応答より遅れても招待の重複送信・重複表示を防ぐ")
    func delayedInvitationSynchronization() async throws {
        let client = FakeCallClient()
        defer { client.beforeInviteResult = nil }
        let store = try await connected(client)
        defer { store.shutdown() }
        let task = await observeInvitations(client, store: store)
        defer { task.cancel() }
        let invitation = CallInvitation(id: "invite", channel_id: channel.id, inviter_id: "me", invitee_id: "other",
                                        status: "pending", expires_at: Date.now.addingTimeInterval(30).ISO8601Format())
        let members = [Person(id: "other", user_name: "other", display_name: "相手")]
        client.inviteResult = [CreatedCallInvitation(id: "invite", invitee_id: "other")]
        client.beforeInviteResult = {
            #expect(!(await store.invite(userIDs: ["other"], members: members, channelID: channel.id)))
        }
        #expect(await store.invite(userIDs: ["other"], members: members, channelID: channel.id))
        #expect(!(await store.invite(userIDs: ["other"], members: members, channelID: channel.id)))
        #expect(client.inviteRequests.count == 1 && store.pendingOutgoing().count == 1)
        #expect(store.pendingOutgoing(at: .now.addingTimeInterval(31)).isEmpty)
        await deliverInvitations([invitation], client: client)
        #expect(store.pendingOutgoing().count == 1)
        await deliverInvitations([], client: client)
        #expect(store.pendingOutgoing().isEmpty)
    }

    @Test("同期前に取り消した招待は古いpending行が届いても復活しない")
    func cancellationBeforeSynchronization() async throws {
        let client = FakeCallClient()
        let store = try await connected(client)
        defer { store.shutdown() }
        let task = await observeInvitations(client, store: store)
        defer { task.cancel() }
        let members = [Person(id: "other", user_name: "other", display_name: "相手")]
        client.inviteResult = [CreatedCallInvitation(id: "invite", invitee_id: "other")]
        #expect(await store.invite(userIDs: ["other"], members: members, channelID: channel.id))
        let invitation = try #require(store.pendingOutgoing().first)
        await store.cancelInvitation(invitation)
        await deliverInvitations([invitation], client: client)
        #expect(store.pendingOutgoing().isEmpty && store.invitationSlots() == 6)
        #expect(store.invitationCandidates(members: members).map(\.id) == ["other"])
        await deliverInvitations([], client: client)
        client.inviteResult = [CreatedCallInvitation(id: "new-invite", invitee_id: "other")]
        #expect(await store.invite(userIDs: ["other"], members: members, channelID: channel.id))
        #expect(store.pendingOutgoing().map(\.id) == ["new-invite"])
    }

    @Test("DMと未接続の通話には手動招待を送らない")
    func rejectsDMAndDisconnected() async throws {
        let client = FakeCallClient()
        var dm = channel
        dm.is_dm = true
        let store = try await connected(client, channel: dm)
        let members = [Person(id: "other", user_name: "other", display_name: "相手")]
        #expect(store.invitationCandidates(members: members).isEmpty)
        #expect(!(await store.invite(userIDs: ["other"], members: members, channelID: channel.id)))
        store.finish()
        #expect(!(await store.invite(userIDs: ["other"], members: members, channelID: channel.id)))
        #expect(client.inviteRequests.isEmpty)
    }

    @Test("同じ会話で通話を切り替えても、成功した招待取消を同期の遅れで復活させない")
    func cancellationSurvivesCallReplacement() async throws {
        let client = FakeCallClient()
        let store = try await connected(client)
        defer { client.beforeCancelResult = nil; store.shutdown() }
        let observing = await observeInvitations(client, store: store)
        defer { observing.cancel() }
        let invitation = CallInvitation(id: "invite", channel_id: channel.id, inviter_id: "me", invitee_id: "other",
                                        status: "pending", expires_at: Date.now.addingTimeInterval(30).ISO8601Format())
        await deliverInvitations([invitation], client: client)
        client.beforeCancelResult = {
            store.finish()
            do { try await waitForCallDelivery(client) { store.start(channel: channel) } }
            catch { Issue.record(error) }
        }
        await store.cancelInvitation(invitation)
        await deliverInvitations([invitation], client: client)
        #expect(store.ready && store.pendingOutgoing().isEmpty)
        #expect(client.cancelledInvitations == [invitation.id])
    }

    @Test("通話を切り替えた後に古い招待の結果を表示しない")
    func ignoresLateInvitationResult() async throws {
        let client = FakeCallClient()
        defer { client.beforeInviteResult = nil }
        let store = try await connected(client)
        defer { store.shutdown() }
        client.inviteResult = [CreatedCallInvitation(id: "invite", invitee_id: "other")]
        client.beforeInviteResult = {
            store.finish()
            await withCheckedContinuation { continuation in
                client.delivered = { continuation.resume() }
                store.start(channel: channel)
            }
            client.delivered = nil
        }
        let members = [Person(id: "other", user_name: "other", display_name: "相手")]
        #expect(!(await store.invite(userIDs: ["other"], members: members, channelID: channel.id)))
        #expect(store.ready && store.invitationNotice == nil && store.invitationError == nil && !store.inviting)
    }

    @Test("サーバーが一部または全員を招待しなかった場合を伝える", arguments: [0, 1])
    func partialInvitations(count: Int) async throws {
        let client = FakeCallClient()
        let store = try await connected(client)
        defer { store.shutdown() }
        let members = ["one", "two"].map { Person(id: $0, user_name: $0, display_name: $0) }
        client.inviteResult = count == 0 ? [] : [CreatedCallInvitation(id: "invite", invitee_id: "one")]
        #expect(await store.invite(userIDs: ["one", "two"], members: members, channelID: channel.id) == (count > 0))
        #expect(store.ready)
        if count == 0 { #expect(store.invitationError != nil) }
        else { #expect(store.invitationNotice?.contains("残りのメンバー") == true) }
    }

    @Test("自分の呼び出しを取り消し、失敗時は再試行できる")
    func cancelInvitation() async throws {
        let client = FakeCallClient()
        let store = try await connected(client)
        defer { store.shutdown() }
        let invitation = CallInvitation(id: "invite", channel_id: channel.id, inviter_id: "me", invitee_id: "other",
                                        status: "pending", expires_at: Date.now.addingTimeInterval(30).ISO8601Format())
        client.invitationRows = [invitation]
        let task = await observeInvitations(client, store: store)
        defer { task.cancel() }
        client.inviteFailure = URLError(.notConnectedToInternet)
        await store.cancelInvitation(invitation)
        #expect(store.pendingOutgoing().count == 1 && store.invitationError != nil && store.ready)
        client.inviteFailure = nil
        await store.cancelInvitation(invitation)
        #expect(store.pendingOutgoing().isEmpty && store.invitationError == nil)
        #expect(client.cancelledInvitations == ["invite"] && store.cancellingInvitations.isEmpty)
    }

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
    func deniedMicrophone() async throws {
        let client = FakeCallClient()
        let media = FakeCallMedia()
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { false })
        await store.start(channel: channel, invitationID: "invite")?.value
        #expect(store.error != nil)
        #expect(!client.observed && !client.accepted && !media.started)
        #expect(store.channel == nil)
    }

    @Test("Siriの発信結果は接続完了後に返し、マイク拒否は失敗として返す", arguments: [false, true])
    func shortcutCallWaitsForConnection(allowed: Bool) async throws {
        let client = FakeCallClient()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let media = FakeCallMedia()
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { allowed })
        defer { store.shutdown() }
        if allowed {
            try await store.startAndWait(channel: channel)
            #expect(store.ready && media.started)
            await #expect(throws: ZooError.self) { try await store.startAndWait(channel: channel) }
            #expect(store.ready)
        } else {
            await #expect(throws: ZooError.self) { try await store.startAndWait(channel: channel) }
            #expect(!store.ready && !media.started && store.channel == nil)
        }
    }

    @Test("Siriの入室待ちをキャンセルしたら接続を終了する")
    func cancelledShortcutCall() async throws {
        let client = FakeCallClient()
        let media = FakeCallMedia()
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { true })
        defer { store.shutdown() }
        let started = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish() }
        client.delivered = { started.continuation.yield(()) }
        let pending = Task { try await store.startAndWait(channel: channel) }
        for await _ in started.stream { break }
        #expect(!media.started)
        pending.cancel()
        await #expect(throws: CancellationError.self) { try await pending.value }
        #expect(store.channel == nil && !store.ready)
    }

    @Test("暗号化チャンネルでも着信に応答して通話へ参加できる")
    func encryptedInvitation() async throws {
        var encrypted = channel
        encrypted.is_private = true
        encrypted.encrypted = true
        #expect(!encrypted.canPost)
        let client = FakeCallClient()
        let media = FakeCallMedia()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let store = CallStore(client: client, userID: "me", makeMedia: { media }, permission: { true })
        defer { store.shutdown() }
        let delivered = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        client.delivered = { delivered.continuation.yield(()); delivered.continuation.finish() }
        _ = try #require(store.start(channel: encrypted, invitationID: "invite", systemAudio: true))
        for await _ in delivered.stream { }
        #expect(client.accepted && client.observed)
        #expect(store.ready && media.started)
        #expect(store.channel?.id == encrypted.id)
    }

    @Test("削除・アーカイブ済みのチャンネルでは着信に応答しない", arguments: [true, false])
    func unavailableChannelInvitation(archived: Bool) {
        var unavailable = channel
        unavailable.encrypted = true
        if archived { unavailable.archive_at = "2026-11-19T00:00:00Z" }
        else { unavailable.deleted_at = "2026-11-19T00:00:00Z" }
        let client = FakeCallClient()
        let store = CallStore(client: client, userID: "me", makeMedia: { FakeCallMedia() }, permission: { true })
        defer { store.shutdown() }
        #expect(store.start(channel: unavailable, invitationID: "invite") == nil)
        #expect(!client.accepted && !client.observed)
    }

    @Test("切断後にメディアを残さず終了し、再入室できる")
    func streamFailure() async throws {
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
    @Test("認証付きで通話中チャンネルの非Envelope一覧を取得する")
    func activeCalls() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/api/calls/active")
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            return (200, [:], Data(#"[{"channel_id":"channel","participant_count":2,"participants":[]}]"#.utf8))
        }
        let calls = try await harness.client.activeCalls()
        #expect(calls.count == 1)
        #expect(calls.first?.channel_id == "channel")
        #expect(calls.first?.participant_count == 2)
    }

    @Test("チャンネル招待は宛先をPOSTし、Envelopeのない招待一覧を受け取る")
    func invite() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/api/channels/11111111-1111-4111-8111-111111111111/call/invites")
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
            #expect(body["user_ids"] == .array([.string("other")]))
            return (200, [:], Data(#"{"invites":[{"id":"invite","invitee_id":"other"}]}"#.utf8))
        }
        let created = try await harness.client.inviteToCall(channelID: "11111111-1111-4111-8111-111111111111", userIDs: ["other"])
        #expect(created.count == 1 && created.first?.invitee_id == "other")
    }

    @Test("取り消し済み・応答済みの招待の取り消しを成功として扱う", arguments: [200, 404, 409])
    func cancel(status: Int) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/api/call-invites/22222222-2222-4222-8222-222222222222/cancel")
            return (status, [:], Data(#"{"ok":true}"#.utf8))
        }
        try await harness.client.cancelCallInvitation(id: "22222222-2222-4222-8222-222222222222")
    }

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
