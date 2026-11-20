import AVFAudio
import Foundation
import Observation

@Observable @MainActor
final class CallStore {
    private let client: any CallClient
    private let userID: String
    private let makeMedia: @MainActor () -> any CallMedia
    private let permission: @MainActor () async -> Bool
    private let retryDelay: Duration
    private(set) var channel: Channel?
    private(set) var participants: [CallParticipant] = []
    private(set) var activeChannelIDs: Set<String> = []
    private var activeCallsGeneration = UUID()
    private(set) var ready = false
    private(set) var peerStates: [String: CallPeerState] = [:]
    private(set) var heartbeatRetrying = false
    private(set) var iceServerKinds: [String] = []
    private(set) var retryChannel: Channel?
    private var retryMuted = false
    private var retrySpeaker = false
    private(set) var diagnostics: [CallPeerDiagnostics] = []
    private(set) var muted = false
    private(set) var speakerEnabled = false
    private(set) var changingMute = false
    private(set) var invitations: [CallInvitation] = []
    private(set) var outgoingInvitations: [CallInvitation] = []
    private var submittedInvitations: [CallInvitation] = []
    private var invitationsSeenWhileSending: Set<String> = []
    private var cancelledInvitationExpirations: [String: Date] = [:]
    private(set) var inviting = false
    private(set) var cancellingInvitations: Set<String> = []
    private(set) var invitationError: String?
    private(set) var invitationNotice: String?
    private(set) var maxParticipants = 6
    var error: String?
    var showingCall = false
    @ObservationIgnored var canStart: ((Bool) -> Bool)?
    @ObservationIgnored var onWillStart: (() -> Void)?
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored var onInvitationsChanged: (([CallInvitation]) -> Void)?
    @ObservationIgnored var onReady: (() -> Void)?
    @ObservationIgnored var prepareSystemCall: ((UUID, Channel) async throws -> Void)?
    @ObservationIgnored var onPeerConnected: (() -> Void)?
    @ObservationIgnored var onFinished: (() -> Void)?
    @ObservationIgnored var onMutedChanged: ((Bool) -> Void)?
    @ObservationIgnored private var media: (any CallMedia)?
    @ObservationIgnored private var connection: Task<Void, Never>?
    @ObservationIgnored private var heartbeat: Task<Void, Never>?
    @ObservationIgnored private var outgoing: Task<Void, Never>?
    @ObservationIgnored private var timeout: Task<Void, Never>?
    @ObservationIgnored private var mutation: Task<Void, Never>?
    @ObservationIgnored private var signals: AsyncStream<(String, CallSignal)>.Continuation?
    private var sessionID: String?
    private var active = true
    private var systemAudioActive = false

    init(client: any CallClient, userID: String,
         makeMedia: @escaping @MainActor () -> any CallMedia = { NativeCallMedia() },
         permission: @escaping @MainActor () async -> Bool = { await AVAudioApplication.requestRecordPermission() },
         retryDelay: Duration = .seconds(1)) {
        self.client = client; self.userID = userID; self.makeMedia = makeMedia; self.permission = permission
        self.retryDelay = retryDelay
    }

    var connectionStatus: String {
        guard ready else { return "接続中" }
        if heartbeatRetrying { return "通信を確認中" }
        let remote = participants.filter { $0.id != sessionID }
        guard !remote.isEmpty else { return "参加者を待機中" }
        let states = remote.map { peerStates[$0.id] ?? .connecting }
        if states.allSatisfy({ $0 == .connected }) { return "通話中" }
        if states.contains(.reconnecting) { return "音声を再接続中" }
        if states.contains(.failed) { return "音声の接続に失敗" }
        return "音声を接続中"
    }

    func refreshDiagnostics() async {
        guard let session = sessionID, let media else { return }
        let result = await media.diagnostics()
        guard !Task.isCancelled, sessionID == session else { return }
        diagnostics = result
    }

    func retry() {
        guard let retryChannel, channel == nil else { return }
        start(channel: retryChannel, initialMuted: retryMuted, initialSpeaker: retrySpeaker)
    }

    func hasActiveCall(in channelID: String) -> Bool {
        activeChannelIDs.contains(channelID) || (ready && channel?.id == channelID)
    }

    func refreshActiveCalls() async {
        guard active, !Task.isCancelled else { return }
        let generation = UUID()
        activeCallsGeneration = generation
        do {
            let calls = try await client.activeCalls()
            try Task.checkCancellation()
            guard active, activeCallsGeneration == generation else { return }
            activeChannelIDs = Set(calls.filter { $0.participant_count > 0 }.map(\.channel_id))
        } catch {
            guard active, !Task.isCancelled, activeCallsGeneration == generation else { return }
            activeChannelIDs = []
            if case ZooError.expiredSession = error { onExpiredSession?() }
        }
    }

    func runActiveCalls() async {
        while active, !Task.isCancelled {
            await refreshActiveCalls()
            do { try await Task.sleep(for: .seconds(10)) } catch { return }
        }
    }

    @discardableResult
    func start(channel: Channel, invitationID: String? = nil, systemAudio: Bool = false,
               initialMuted: Bool = false, initialSpeaker: Bool = false) -> Task<Void, Never>? {
        guard active, self.channel == nil, channel.canCall, canStart?(systemAudio) != false else { return nil }
        onWillStart?()
        let callID = UUID()
        let session = callID.uuidString.lowercased()
        self.channel = channel; sessionID = session
        muted = initialMuted; speakerEnabled = initialSpeaker
        error = nil; retryChannel = nil; showingCall = true
        let prepareSystemCall = systemAudio ? nil : self.prepareSystemCall
        let usesSystemAudio = systemAudio || prepareSystemCall != nil
        connection = Task { [weak self] in
            guard let self else { return }
            do {
                guard await permission() else { throw ZooError.rejected(403, "設定でマイクへのアクセスを許可してください。") }
                try Task.checkCancellation()
                guard sessionID == session else { return }
                if let prepareSystemCall { try await prepareSystemCall(callID, channel) }
                try Task.checkCancellation()
                guard sessionID == session else { return }
                if let invitationID { try await client.answerCallInvitation(id: invitationID, accept: true) }
                try Task.checkCancellation()
                guard sessionID == session else { return }
                timeout = Task { [weak self] in
                    do { try await Task.sleep(for: .seconds(20)) } catch { return }
                    guard let self, sessionID == session, !ready else { return }
                    fail(URLError(.timedOut), session: session)
                }
                var early: [CallEvent] = []
                try await client.observeCall(channelID: channel.id, sessionID: session) { [weak self] event in
                    guard let self, sessionID == session, !Task.isCancelled else { throw CancellationError() }
                    if case .room(let room) = event {
                        guard !ready, room.participants.count <= 6 else { throw ZooError.invalidResponse }
                        let engine = makeMedia()
                        (engine as? NativeCallMedia)?.systemAudio = usesSystemAudio
                        try (engine as? NativeCallMedia)?.setSystemAudioActive(systemAudioActive)
                        media = engine
                        engine.onPeerState = { [weak self] id, state in
                            guard let self, sessionID == session else { return }
                            peerStates[id] = state
                            if state == .connected { onPeerConnected?() }
                        }
                        prepareSignals(channelID: channel.id, session: session)
                        try engine.start(sessionID: session, room: room) { [weak self] target, signal in
                            guard let self, sessionID == session else { return }
                            if case .dropped = signals?.yield((target, signal)) { fail(ZooError.invalidResponse, session: session) }
                        } failed: { [weak self] in self?.fail(URLError(.cannotConnectToHost), session: session) }
                        // 再参加でミュートが解除されないよう、相手へtrackを追加する前に適用する。
                        engine.mute(muted)
                        if speakerEnabled { try engine.speaker(true) }
                        if muted { try await sendMute(channelID: channel.id, session: session, value: true) }
                        try Task.checkCancellation()
                        guard sessionID == session else { return }
                        let departed = Set(early.compactMap { event -> String? in
                            if case .left(let id) = event { return id }; return nil
                        })
                        let initial = CallRoom(participants: room.participants.filter { !departed.contains($0.id) },
                                               max_participants: room.max_participants, ice_servers: room.ice_servers)
                        for id in departed { try await engine.handle(.left(id)) }
                        ready = true
                        iceServerKinds = Array(Set(room.ice_servers.flatMap(\.urls).compactMap {
                            let kind = String($0.prefix(while: { $0 != ":" })).lowercased()
                            return ["stun", "turn", "turns"].contains(kind) ? kind.uppercased() : nil
                        })).sorted()
                        maxParticipants = min(6, max(1, room.max_participants))
                        timeout?.cancel(); timeout = nil
                        participants = initial.participants
                        try await engine.handle(.room(initial))
                        for pending in early { try await receive(pending, session: session) }
                        early = []
                        startHeartbeat(channelID: channel.id, session: session)
                        // CallKitの応答完了で音声セッションが有効になるため、入室と音声接続を分ける。
                        onReady?()
                    } else if ready { try await receive(event, session: session) }
                    else {
                        guard early.count < 256 else { throw ZooError.invalidResponse }
                        early.append(event)
                    }
                }
                throw URLError(.networkConnectionLost)
            } catch {
                guard !Task.isCancelled else { return }
                fail(error, session: session)
            }
        }
        return connection
    }

    func startAndWait(channel: Channel) async throws {
        guard start(channel: channel) != nil, let requestedSession = sessionID else {
            throw ZooError.auth("別の通話や着信を終了してから、もう一度お試しください。")
        }
        do {
            while sessionID == requestedSession, !ready {
                try await Task.sleep(for: .milliseconds(100))
            }
            try Task.checkCancellation()
            guard sessionID == requestedSession, ready else {
                throw ZooError.auth(error ?? "通話を開始できませんでした。")
            }
        } catch {
            if sessionID == requestedSession { finish() }
            throw error
        }
    }

    private func receive(_ event: CallEvent, session: String) async throws {
        guard sessionID == session else { throw CancellationError() }
        switch event {
        case .joined(let participant), .updated(let participant):
            participants.removeAll { $0.id == participant.id }
            participants.append(participant)
        case .left(let id):
            if id == session { finish(); return }
            participants.removeAll { $0.id == id }
            peerStates[id] = nil
        default: break
        }
        try await media?.handle(event)
    }

    private func prepareSignals(channelID: String, session: String) {
        let (stream, continuation) = AsyncStream<(String, CallSignal)>.makeStream(bufferingPolicy: .bufferingOldest(256))
        signals = continuation
        outgoing = Task { [weak self] in
            guard let self else { return }
            do {
                for await (target, original) in stream {
                    try Task.checkCancellation()
                    guard sessionID == session else { return }
                    guard participants.contains(where: { $0.id == target }) else { continue }
                    var signal = original
                    signal.signal_id = UUID().uuidString.lowercased()
                    let body: [String: JSONValue] = ["session_id": .string(session), "target_session_id": .string(target), "signal": try signal.json()]
                    for attempt in 0..<3 {
                        guard !Task.isCancelled, sessionID == session else { return }
                        do {
                            try await client.callCommand(channelID: channelID, action: "signal", body: body)
                            break
                        } catch {
                            guard !Task.isCancelled, sessionID == session else { return }
                            if case ZooError.rejected(404, let detail) = error,
                               !participants.contains(where: { $0.id == target }) || detail.hasPrefix("Target participant") {
                                participants.removeAll { $0.id == target }
                                try await media?.handle(.left(target))
                                break
                            }
                            if CallRetryPolicy.isTransient(error), attempt < 2 {
                                try await Task.sleep(for: retryDelay * (attempt + 1))
                            } else { throw error }
                        }
                    }
                }
            } catch { if !Task.isCancelled { fail(error, session: session) } }
        }
    }

    private func startHeartbeat(channelID: String, session: String) {
        heartbeat = Task { [weak self] in
            guard let self else { return }
            do {
                while sessionID == session, !Task.isCancelled {
                    // presenceの有効期間は30秒。短い障害だけを1秒・2秒の待機で再試行する。
                    for attempt in 0..<3 {
                        do {
                            try await client.callCommand(channelID: channelID, action: "heartbeat", body: ["session_id": .string(session)])
                            guard sessionID == session, !Task.isCancelled else { return }
                            heartbeatRetrying = false
                            break
                        } catch {
                            guard sessionID == session, !Task.isCancelled else { return }
                            guard CallRetryPolicy.isTransient(error), attempt < 2 else { throw error }
                            heartbeatRetrying = true
                            try await Task.sleep(for: retryDelay * (attempt + 1))
                        }
                    }
                    try await Task.sleep(for: .seconds(10))
                }
            } catch { if !Task.isCancelled { fail(error, session: session) } }
        }
    }

    func toggleMute() {
        guard ready, !changingMute, let channel, let session = sessionID else { return }
        muted.toggle(); media?.mute(muted); changingMute = true
        onMutedChanged?(muted)
        let value = muted
        mutation = Task { [weak self] in
            guard let self else { return }
            do {
                try await sendMute(channelID: channel.id, session: session, value: value)
                if sessionID == session { changingMute = false }
            } catch { if !Task.isCancelled { fail(error, session: session) } }
        }
    }

    private func sendMute(channelID: String, session: String, value: Bool) async throws {
        for attempt in 0..<3 {
            try Task.checkCancellation()
            guard sessionID == session else { throw CancellationError() }
            do {
                try await client.callCommand(channelID: channelID, action: "state",
                                             body: ["session_id": .string(session), "is_muted": .bool(value)])
                return
            } catch {
                guard CallRetryPolicy.isTransient(error), attempt < 2 else { throw error }
                try await Task.sleep(for: retryDelay * (attempt + 1))
            }
        }
    }

    func toggleSpeaker() {
        guard ready else { return }
        do { try media?.speaker(!speakerEnabled); speakerEnabled.toggle() }
        catch { self.error = "音声の出力先を変更できませんでした。" }
    }

    func setSystemAudioActive(_ active: Bool) {
        systemAudioActive = active
        do { try (media as? NativeCallMedia)?.setSystemAudioActive(active) }
        catch { if let sessionID { fail(error, session: sessionID) } }
    }

    func finish() {
        let oldChannel = channel?.id
        let oldSession = sessionID
        sessionID = nil
        systemAudioActive = false
        connection?.cancel(); connection = nil
        heartbeat?.cancel(); heartbeat = nil
        outgoing?.cancel(); outgoing = nil
        timeout?.cancel(); timeout = nil
        mutation?.cancel(); mutation = nil
        signals?.finish(); signals = nil
        media?.close(); media = nil
        channel = nil; participants = []; ready = false; muted = false; speakerEnabled = false; changingMute = false
        peerStates = [:]; diagnostics = []; iceServerKinds = []; heartbeatRetrying = false; retryChannel = nil
        retryMuted = false; retrySpeaker = false
        inviting = false; cancellingInvitations = []; invitationError = nil; invitationNotice = nil
        submittedInvitations = []; invitationsSeenWhileSending = []
        if let oldChannel, let oldSession {
            onFinished?()
            let client = client
            Task { try? await client.callCommand(channelID: oldChannel, action: "leave", body: ["session_id": .string(oldSession)]) }
        }
    }

    private func fail(_ error: Error, session: String) {
        guard sessionID == session else { return }
        let previousChannel = channel
        let previousMuted = muted
        let previousSpeaker = speakerEnabled
        finish()
        if CallRetryPolicy.isTransient(error) {
            retryChannel = previousChannel
            retryMuted = previousMuted; retrySpeaker = previousSpeaker
            self.error = "通話の接続が途切れました。通信環境を確認して再接続してください。"
        } else { self.error = "通話を続けられませんでした。\(error.localizedDescription)" }
        if case ZooError.expiredSession = error { onExpiredSession?() }
    }

    func runInvitations() async {
        while active, !Task.isCancelled {
            do {
                try await client.observeCallInvitations { [weak self] values in
                    guard let self, active, !Task.isCancelled else { return }
                    invitations = values.filter { $0.invitee_id == userID && $0.status == "pending" }
                    onInvitationsChanged?(invitations)
                    cancelledInvitationExpirations = cancelledInvitationExpirations.filter { $0.value > .now }
                    outgoingInvitations = values.filter {
                        $0.inviter_id == userID && $0.status == "pending" && cancelledInvitationExpirations[$0.id] == nil
                    }
                    let receivedIDs = Set(values.map(\.id))
                    if inviting { invitationsSeenWhileSending.formUnion(receivedIDs) }
                    submittedInvitations.removeAll { receivedIDs.contains($0.id) || (ZooDate.parse($0.expires_at) ?? .distantPast) <= .now }
                }
            } catch {
                guard active, !Task.isCancelled else { return }
                invitations = []
                outgoingInvitations = []
                if case ZooError.expiredSession = error { onExpiredSession?(); return }
            }
            do { try await Task.sleep(for: .seconds(3)) } catch { return }
        }
    }

    func decline(_ invitation: CallInvitation) async {
        do {
            try await client.answerCallInvitation(id: invitation.id, accept: false)
            guard active, !Task.isCancelled else { return }
            invitations.removeAll { $0.id == invitation.id }
        } catch {
            guard active, !Task.isCancelled else { return }
            self.error = "通話への応答を送れませんでした。"
            if case ZooError.expiredSession = error { onExpiredSession?() }
        }
    }

    func pendingOutgoing(at date: Date = .now) -> [CallInvitation] {
        (outgoingInvitations + submittedInvitations).filter { invitation in
            invitation.channel_id == channel?.id && (ZooDate.parse(invitation.expires_at) ?? .distantPast) > date &&
            !participants.contains(where: { $0.user_id == invitation.invitee_id })
        }
    }

    func invitationSlots(at date: Date = .now) -> Int {
        max(0, maxParticipants - participants.count - Set(pendingOutgoing(at: date).map(\.invitee_id)).count)
    }

    func invitationCandidates(members: [Person], at date: Date = .now) -> [Person] {
        guard ready, channel?.is_dm == false else { return [] }
        let unavailable = Set(participants.map(\.user_id) + pendingOutgoing(at: date).map(\.invitee_id) + [userID])
        return members.filter { !$0.is_bot && !$0.is_deactivated && !unavailable.contains($0.id) }
    }

    func invite(userIDs: Set<String>, members: [Person], channelID: String) async -> Bool {
        guard active, ready, !inviting, let channel, channel.id == channelID, !channel.is_dm,
              let session = sessionID else { return false }
        let candidates = Set(invitationCandidates(members: members).map(\.id))
        guard !userIDs.isEmpty, userIDs.isSubset(of: candidates), userIDs.count <= invitationSlots() else {
            invitationError = "参加者や空き人数が変わりました。招待するメンバーを選び直してください。"
            return false
        }
        inviting = true; invitationError = nil; invitationNotice = nil
        invitationsSeenWhileSending = []
        submittedInvitations.removeAll { (ZooDate.parse($0.expires_at) ?? .distantPast) <= .now }
        defer { if sessionID == session { inviting = false } }
        do {
            let created = try await client.inviteToCall(channelID: channelID, userIDs: userIDs.sorted())
            guard active, sessionID == session, !Task.isCancelled else { return false }
            guard !created.isEmpty else {
                invitationError = "招待できませんでした。満員、参加済み、または呼び出し中の可能性があります。"
                return false
            }
            // Shapeが遅れても取り消せるように返却IDを保持する。応答前に観測済みの招待は復活させない。
            submittedInvitations += created.filter { !invitationsSeenWhileSending.contains($0.id) }.map {
                CallInvitation(id: $0.id, channel_id: channelID, inviter_id: userID, invitee_id: $0.invitee_id,
                               status: "pending", expires_at: Date.now.addingTimeInterval(30).ISO8601Format())
            }
            invitationNotice = created.count == userIDs.count ? "\(created.count)人に招待を送りました。" :
                "\(created.count)人に招待を送りました。残りのメンバーは満員、参加済み、または呼び出し中のため招待できませんでした。"
            return true
        } catch {
            guard active, sessionID == session, !Task.isCancelled else { return false }
            invitationError = "招待を送れませんでした。接続を確認して、もう一度お試しください。"
            if case ZooError.expiredSession = error { onExpiredSession?() }
            return false
        }
    }

    func cancelInvitation(_ invitation: CallInvitation) async {
        guard active, ready, invitation.channel_id == channel?.id, invitation.inviter_id == userID,
              !cancellingInvitations.contains(invitation.id), let session = sessionID else { return }
        cancellingInvitations.insert(invitation.id); invitationError = nil
        defer { if sessionID == session { cancellingInvitations.remove(invitation.id) } }
        do {
            try await client.cancelCallInvitation(id: invitation.id)
            guard active, sessionID == session, !Task.isCancelled else { return }
            // 取消APIより遅れて届くpending行で、終了した呼び出しを再表示しない。
            cancelledInvitationExpirations = cancelledInvitationExpirations.filter { $0.value > .now }
            cancelledInvitationExpirations[invitation.id] = ZooDate.parse(invitation.expires_at) ?? .now.addingTimeInterval(30)
            outgoingInvitations.removeAll { $0.id == invitation.id }
            submittedInvitations.removeAll { $0.id == invitation.id }
        } catch {
            guard active, sessionID == session, !Task.isCancelled else { return }
            invitationError = "呼び出しを取り消せませんでした。もう一度お試しください。"
            if case ZooError.expiredSession = error { onExpiredSession?() }
        }
    }

    func shutdown() {
        active = false; finish(); invitations = []; outgoingInvitations = []; cancelledInvitationExpirations = [:]
        activeChannelIDs = []
        error = nil; showingCall = false
    }
}
