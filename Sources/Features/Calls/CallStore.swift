import AVFAudio
import Foundation
import Observation

@Observable @MainActor
final class CallStore {
    private let client: any CallClient
    private let userID: String
    private let makeMedia: @MainActor () -> any CallMedia
    private let permission: @MainActor () async -> Bool
    private(set) var channel: Channel?
    private(set) var participants: [CallParticipant] = []
    private(set) var ready = false
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
    @ObservationIgnored var onWillStart: (() -> Void)?
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    @ObservationIgnored private var media: (any CallMedia)?
    @ObservationIgnored private var connection: Task<Void, Never>?
    @ObservationIgnored private var heartbeat: Task<Void, Never>?
    @ObservationIgnored private var outgoing: Task<Void, Never>?
    @ObservationIgnored private var timeout: Task<Void, Never>?
    @ObservationIgnored private var mutation: Task<Void, Never>?
    @ObservationIgnored private var signals: AsyncStream<(String, CallSignal)>.Continuation?
    private var sessionID: String?
    private var active = true

    init(client: any CallClient, userID: String,
         makeMedia: @escaping @MainActor () -> any CallMedia = { NativeCallMedia() },
         permission: @escaping @MainActor () async -> Bool = { await AVAudioApplication.requestRecordPermission() }) {
        self.client = client; self.userID = userID; self.makeMedia = makeMedia; self.permission = permission
    }

    @discardableResult
    func start(channel: Channel, invitationID: String? = nil) -> Task<Void, Never>? {
        guard active, self.channel == nil, channel.canPost else { return nil }
        onWillStart?()
        let session = UUID().uuidString.lowercased()
        self.channel = channel; sessionID = session
        error = nil; showingCall = true
        connection = Task { [weak self] in
            guard let self else { return }
            do {
                guard await permission() else { throw ZooError.rejected(403, "設定でマイクへのアクセスを許可してください。") }
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
                        media = engine
                        prepareSignals(channelID: channel.id, session: session)
                        try engine.start(sessionID: session, room: room) { [weak self] target, signal in
                            guard let self, sessionID == session else { return }
                            if case .dropped = signals?.yield((target, signal)) { fail(ZooError.invalidResponse, session: session) }
                        } failed: { [weak self] in self?.fail(URLError(.cannotConnectToHost), session: session) }
                        let departed = Set(early.compactMap { event -> String? in
                            if case .left(let id) = event { return id }; return nil
                        })
                        let initial = CallRoom(participants: room.participants.filter { !departed.contains($0.id) },
                                               max_participants: room.max_participants, ice_servers: room.ice_servers)
                        for id in departed { try await engine.handle(.left(id)) }
                        ready = true
                        maxParticipants = min(6, max(1, room.max_participants))
                        timeout?.cancel(); timeout = nil
                        participants = initial.participants
                        try await engine.handle(.room(initial))
                        for pending in early { try await receive(pending, session: session) }
                        early = []
                        startHeartbeat(channelID: channel.id, session: session)
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

    private func receive(_ event: CallEvent, session: String) async throws {
        guard sessionID == session else { throw CancellationError() }
        switch event {
        case .joined(let participant), .updated(let participant):
            participants.removeAll { $0.id == participant.id }
            participants.append(participant)
        case .left(let id):
            if id == session { finish(); return }
            participants.removeAll { $0.id == id }
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
                            if case ZooError.rejected(503, _) = error, attempt < 2 {
                                try await Task.sleep(for: .milliseconds(300 * (attempt + 1)))
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
                    try await client.callCommand(channelID: channelID, action: "heartbeat", body: ["session_id": .string(session)])
                    try await Task.sleep(for: .seconds(10))
                }
            } catch { if !Task.isCancelled { fail(error, session: session) } }
        }
    }

    func toggleMute() {
        guard ready, !changingMute, let channel, let session = sessionID else { return }
        muted.toggle(); media?.mute(muted); changingMute = true
        let value = muted
        mutation = Task { [weak self] in
            guard let self else { return }
            do {
                try await client.callCommand(channelID: channel.id, action: "state", body: ["session_id": .string(session), "is_muted": .bool(value)])
                if sessionID == session { changingMute = false }
            } catch { if !Task.isCancelled { fail(error, session: session) } }
        }
    }

    func toggleSpeaker() {
        guard ready else { return }
        do { try media?.speaker(!speakerEnabled); speakerEnabled.toggle() }
        catch { self.error = "音声の出力先を変更できませんでした。" }
    }

    func finish() {
        let oldChannel = channel?.id
        let oldSession = sessionID
        sessionID = nil
        connection?.cancel(); connection = nil
        heartbeat?.cancel(); heartbeat = nil
        outgoing?.cancel(); outgoing = nil
        timeout?.cancel(); timeout = nil
        mutation?.cancel(); mutation = nil
        signals?.finish(); signals = nil
        media?.close(); media = nil
        channel = nil; participants = []; ready = false; muted = false; speakerEnabled = false; changingMute = false
        inviting = false; cancellingInvitations = []; invitationError = nil; invitationNotice = nil
        submittedInvitations = []; invitationsSeenWhileSending = []
        if let oldChannel, let oldSession {
            let client = client
            Task { try? await client.callCommand(channelID: oldChannel, action: "leave", body: ["session_id": .string(oldSession)]) }
        }
    }

    private func fail(_ error: Error, session: String) {
        guard sessionID == session else { return }
        finish()
        self.error = "通話を続けられませんでした。\(error.localizedDescription)"
        if case ZooError.expiredSession = error { onExpiredSession?() }
    }

    func runInvitations() async {
        while active, !Task.isCancelled {
            do {
                try await client.observeCallInvitations { [weak self] values in
                    guard let self, active, !Task.isCancelled else { return }
                    invitations = values.filter { $0.invitee_id == userID && $0.status == "pending" }
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
        error = nil; showingCall = false
    }
}
