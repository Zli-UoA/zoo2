import AVFAudio
import Foundation
@preconcurrency import WebRTC

@MainActor
protocol CallMedia: AnyObject {
    var onPeerState: (@MainActor (String, CallPeerState) -> Void)? { get set }
    func start(sessionID: String, room: CallRoom, signal: @escaping @MainActor (String, CallSignal) -> Void,
               failed: @escaping @MainActor () -> Void) throws
    func handle(_ event: CallEvent) async throws
    func mute(_ muted: Bool)
    func speaker(_ enabled: Bool) throws
    func diagnostics() async -> [CallPeerDiagnostics]
    func close()
}

@MainActor
protocol CallWaitingAudio: AnyObject {
    func start(failed: @escaping @MainActor () -> Void) throws
    func stop()
}

@MainActor
final class NativeCallWaitingAudio: CallWaitingAudio {
    private let onInput: (@Sendable () -> Void)?
    private var engine: AVAudioEngine?
    private var observer: (any NSObjectProtocol)?
    private var generation = UUID()
    var isRunning: Bool { engine?.isRunning == true }

    init(onInput: (@Sendable () -> Void)? = nil) { self.onInput = onInput }

    func start(failed: @escaping @MainActor () -> Void) throws {
        guard engine == nil else { return }
        let engine = AVAudioEngine()
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else { throw ZooError.invalidResponse }
        // 待機中も通話の音声入力を稼働させる。音声は保存・送信せず、WebRTCの音声入出力開始時に譲る。
        // AVFAudioの音声スレッドで呼ばれるため、MainActorの隔離を継承させない。
        let onInput = onInput
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { @Sendable _, _ in onInput?() }
        self.engine = engine
        do { try engine.start() }
        catch { stop(); throw error }
        let epoch = generation
        observer = NotificationCenter.default.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self, generation == epoch, self.engine != nil else { return }
                // Bluetoothの接続などで入力形式が変わった場合も、待機中の音声入力を再構築する。
                stop()
                do { try start(failed: failed) }
                catch { failed() }
            }
        }
    }

    func stop() {
        generation = UUID()
        if let observer { NotificationCenter.default.removeObserver(observer) }
        observer = nil
        engine?.stop()
        engine?.inputNode.removeTap(onBus: 0)
        engine = nil
    }
}

final class WebRTCCallAudioObserver: NSObject, RTCAudioSessionDelegate {
    let changed: @MainActor @Sendable (Bool) -> Void
    init(changed: @escaping @MainActor @Sendable (Bool) -> Void) { self.changed = changed }
    func audioSessionDidStartPlayOrRecord(_ session: RTCAudioSession) {
        Task { @MainActor [changed] in changed(true) }
    }
    func audioSessionDidStopPlayOrRecord(_ session: RTCAudioSession) {
        Task { @MainActor [changed] in changed(false) }
    }
}

@MainActor
final class NativeCallMedia: NSObject, CallMedia {
    var systemAudio = false
    var onPeerState: (@MainActor (String, CallPeerState) -> Void)?
    private final class Peer {
        let connection: RTCPeerConnection
        var candidates: [CallSignal.Candidate] = []
        var lastAnswer: (offer: String, answer: String)?
        var state: CallPeerState = .connecting
        var recovery: Task<Void, Never>?
        var recoveryID = UUID()
        var restartCount = 0
        var sendingDescription = false
        var localCandidates: [CallSignal.Candidate] = []
        var ignoringOffer = false
        init(_ connection: RTCPeerConnection) { self.connection = connection }
    }
    private let factory = RTCPeerConnectionFactory(encoderFactory: nil, decoderFactory: nil)
    private let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)
    private var peers: [String: Peer] = [:]
    private var early: [String: [CallSignal.Candidate]] = [:]
    private var left: Set<String> = []
    private var iceServers: [RTCIceServer] = []
    private var track: RTCAudioTrack?
    private var sessionID = ""
    private var generation = UUID()
    private var audioActive = false
    private var systemAudioActive = false
    private let waitingAudio: any CallWaitingAudio
    private var webRTCAudioRunning = false
    private(set) var audioObserver: WebRTCCallAudioObserver?
    private var mediaOperation: Task<Void, Error>?
    private var signal: (@MainActor (String, CallSignal) -> Void)?
    private var failed: (@MainActor () -> Void)?
    private let connectionTimeout: Duration
    private let restartTimeout: Duration

    init(connectionTimeout: Duration = .seconds(20), restartTimeout: Duration = .seconds(12),
         waitingAudio: any CallWaitingAudio = NativeCallWaitingAudio()) {
        self.connectionTimeout = connectionTimeout
        self.restartTimeout = restartTimeout
        self.waitingAudio = waitingAudio
        super.init()
    }

    private static var audioConfiguration: RTCAudioSessionConfiguration {
        let configuration = RTCAudioSessionConfiguration.webRTC()
        configuration.category = AVAudioSession.Category.playAndRecord.rawValue
        configuration.mode = AVAudioSession.Mode.voiceChat.rawValue
        configuration.categoryOptions = [.allowBluetoothHFP]
        return configuration
    }

    static func prepareSystemAudio() throws {
        let audio = RTCAudioSession.sharedInstance()
        audio.useManualAudio = true
        audio.isAudioEnabled = false
        audio.lockForConfiguration()
        defer { audio.unlockForConfiguration() }
        try audio.setConfiguration(audioConfiguration)
    }

    func setSystemAudioActive(_ active: Bool) throws {
        systemAudioActive = active
        try updateWaitingAudio()
    }

    private func updateWaitingAudio() throws {
        if track != nil, !webRTCAudioRunning, systemAudio ? systemAudioActive : audioActive {
            try waitingAudio.start { [weak self] in self?.failed?() }
        } else { waitingAudio.stop() }
    }

    func start(sessionID: String, room: CallRoom, signal: @escaping @MainActor (String, CallSignal) -> Void,
               failed: @escaping @MainActor () -> Void) throws {
        self.sessionID = sessionID
        self.signal = signal
        self.failed = failed
        generation = UUID()
        let epoch = generation
        webRTCAudioRunning = false
        let observer = WebRTCCallAudioObserver { [weak self] running in
            guard let self, generation == epoch, track != nil else { return }
            webRTCAudioRunning = running && !peers.isEmpty
            do { try updateWaitingAudio() }
            catch { self.failed?() }
        }
        audioObserver = observer
        iceServers = room.ice_servers.map { RTCIceServer(urlStrings: $0.urls, username: $0.username, credential: $0.credential) }
        let audio = RTCAudioSession.sharedInstance()
        audio.add(observer)
        if !systemAudio { audio.useManualAudio = false }
        audio.lockForConfiguration()
        let configuration = Self.audioConfiguration
        do {
            if systemAudio { try audio.setConfiguration(configuration) }
            else { try audio.setConfiguration(configuration, active: true) }
        } catch { audio.unlockForConfiguration(); throw error }
        audio.unlockForConfiguration()
        audioActive = !systemAudio
        track = factory.audioTrack(withTrackId: "zoo-audio")
        try updateWaitingAudio()
    }

    private func peer(_ id: String) throws -> Peer {
        if let peer = peers[id] { return peer }
        guard track != nil, id != sessionID, UUID(uuidString: id) != nil, !left.contains(id), peers.count < 5 else {
            throw ZooError.invalidResponse
        }
        let config = RTCConfiguration()
        config.iceServers = iceServers
        config.sdpSemantics = .unifiedPlan
        guard let connection = factory.peerConnection(with: config, constraints: constraints, delegate: self), let track else {
            throw ZooError.invalidResponse
        }
        connection.add(track, streamIds: ["zoo-stream"])
        let peer = Peer(connection)
        peer.candidates = early.removeValue(forKey: id) ?? []
        peers[id] = peer
        onPeerState?(id, .connecting)
        scheduleRecovery(id, delay: connectionTimeout)
        return peer
    }

    private func join(_ id: String) async throws {
        guard id != sessionID, !left.contains(id), peers[id] == nil else { return }
        let epoch = generation
        let peer = try peer(id)
        guard sessionID < id else { return }
        peer.sendingDescription = true
        let sdp = try await description(peer.connection, offer: true)
        try await setDescription(peer.connection, sdp: sdp, type: .offer, local: true)
        guard generation == epoch else { return }
        sendDescription(id, peer: peer, type: "offer", sdp: sdp)
    }

    func handle(_ event: CallEvent) async throws {
        try await enqueue { [weak self] in try await self?.perform(event) }
    }

    private func enqueue(_ action: @escaping @MainActor () async throws -> Void) async throws {
        let previous = mediaOperation
        let epoch = generation
        // SSEと送信失敗の双方から退出が来ても、実行中のSDP操作にcloseを割り込ませない。
        let operation = Task { @MainActor [weak self] in
            _ = try? await previous?.value
            guard let self, generation == epoch else { return }
            try Task.checkCancellation()
            try await action()
        }
        mediaOperation = operation
        try await withTaskCancellationHandler {
            try await operation.value
        } onCancel: { operation.cancel() }
    }

    private func perform(_ event: CallEvent) async throws {
        guard track != nil else { return }
        let epoch = generation
        switch event {
        case .room(let room):
            for participant in room.participants {
                try Task.checkCancellation()
                try await join(participant.session_id)
            }
        case .joined(let participant): try await join(participant.session_id)
        case .updated: break
        case .left(let id):
            left.insert(id)
            let removed = peers.removeValue(forKey: id)
            removed?.recovery?.cancel()
            removed?.connection.close()
            early[id] = nil
            if peers.isEmpty {
                webRTCAudioRunning = false
                try updateWaitingAudio()
            }
        case .signal(let id, let value):
            guard id != sessionID, !left.contains(id), UUID(uuidString: id) != nil else { return }
            switch value.type {
            case "offer":
                guard let sdp = value.sdp else { throw ZooError.invalidResponse }
                let peer = try peer(id)
                if let cached = peer.lastAnswer, cached.offer == sdp {
                    signal?(id, CallSignal(type: "answer", sdp: cached.answer)); return
                }
                // 更新済みiOS同士が同時に再接続しても、片側だけが自分のofferを取り下げる。
                if peer.connection.signalingState == .haveLocalOffer {
                    guard sessionID > id else { peer.ignoringOffer = true; return }
                    try await setDescription(peer.connection, sdp: "", type: .rollback, local: true)
                    peer.localCandidates = []
                }
                peer.ignoringOffer = false
                peer.sendingDescription = true
                try await setDescription(peer.connection, sdp: sdp, type: .offer, local: false)
                try await flush(peer)
                let answer = try await description(peer.connection, offer: false)
                try await setDescription(peer.connection, sdp: answer, type: .answer, local: true)
                guard epoch == generation else { return }
                peer.lastAnswer = (sdp, answer)
                sendDescription(id, peer: peer, type: "answer", sdp: answer)
            case "answer":
                guard let sdp = value.sdp, let peer = peers[id], peer.connection.signalingState == .haveLocalOffer else { return }
                try await setDescription(peer.connection, sdp: sdp, type: .answer, local: false)
                peer.ignoringOffer = false
                try await flush(peer)
            case "candidate":
                guard let candidate = value.candidate else { return }
                if let peer = peers[id] {
                    guard !peer.ignoringOffer else { return }
                    if peer.connection.remoteDescription != nil { try await add(candidate, to: peer.connection) }
                    else if peer.candidates.count < 128 { peer.candidates.append(candidate) }
                } else if early.count < 6, (early[id]?.count ?? 0) < 128 {
                    early[id, default: []].append(candidate)
                }
            default: break
            }
        }
    }

    private func sendDescription(_ id: String, peer: Peer, type: String, sdp: String) {
        signal?(id, CallSignal(type: type, sdp: sdp))
        peer.sendingDescription = false
        // シグナル送信は直列。SDPのPOSTが成功するまで新しいICE候補を対向に送らない。
        let candidates = peer.localCandidates
        peer.localCandidates = []
        for candidate in candidates { signal?(id, CallSignal(type: "candidate", candidate: candidate)) }
    }

    private func scheduleRecovery(_ id: String, delay: Duration) {
        guard let peer = peers[id], peer.recovery == nil else { return }
        let epoch = generation
        let recoveryID = UUID()
        peer.recoveryID = recoveryID
        peer.recovery = Task { [weak self] in
            do {
                try await Task.sleep(for: delay)
                guard let self, generation == epoch, peers[id] === peer else { return }
                try await enqueue { [weak self] in
                    guard let self, generation == epoch, peers[id] === peer,
                          peer.recoveryID == recoveryID else { return }
                    peer.recovery = nil
                    if peer.connection.connectionState == .connected { return }
                    guard peer.restartCount < 2, peer.connection.remoteDescription != nil,
                          peer.connection.signalingState == .stable else {
                        peer.state = .failed
                        onPeerState?(id, .failed)
                        failed?()
                        return
                    }
                    peer.state = .reconnecting
                    onPeerState?(id, .reconnecting)
                    peer.sendingDescription = true
                    defer {
                        if generation == epoch, peers[id] === peer, peer.sendingDescription {
                            peer.sendingDescription = false
                            peer.localCandidates.removeAll()
                        }
                    }
                    peer.connection.restartIce()
                    let sdp = try await description(peer.connection, offer: true)
                    guard generation == epoch, peers[id] === peer, peer.recoveryID == recoveryID else { return }
                    try await setDescription(peer.connection, sdp: sdp, type: .offer, local: true)
                    guard generation == epoch, peers[id] === peer else { return }
                    if peer.recoveryID != recoveryID {
                        // 接続復旧で不要になった未送信offerを残すと、次の交渉を開始できない。
                        if peer.connection.signalingState == .haveLocalOffer {
                            try await setDescription(peer.connection, sdp: "", type: .rollback, local: true)
                        }
                        return
                    }
                    peer.restartCount += 1
                    sendDescription(id, peer: peer, type: "offer", sdp: sdp)
                    scheduleRecovery(id, delay: restartTimeout)
                }
            } catch {
                guard !Task.isCancelled, let self, generation == epoch, peers[id] === peer, peer.recoveryID == recoveryID else { return }
                failed?()
            }
        }
    }

    private func connectionChanged(_ connection: RTCPeerConnection) {
        guard let (id, peer) = peers.first(where: { $0.value.connection === connection }) else { return }
        switch connection.connectionState {
        case .connected:
            peer.recoveryID = UUID()
            peer.recovery?.cancel(); peer.recovery = nil
            peer.restartCount = 0
            peer.state = .connected
        case .disconnected, .failed:
            peer.state = .reconnecting
            scheduleRecovery(id, delay: .seconds(3))
        default: break
        }
        onPeerState?(id, peer.state)
    }

    func diagnostics() async -> [CallPeerDiagnostics] {
        let epoch = generation
        var results: [CallPeerDiagnostics] = []
        let observedPeers = peers
        for (id, peer) in observedPeers.sorted(by: { $0.key < $1.key }) {
            let state = peer.state
            let ice: String
            switch peer.connection.iceConnectionState {
            case .new: ice = "未接続"
            case .checking: ice = "確認中"
            case .connected, .completed: ice = "接続済み"
            case .disconnected: ice = "切断"
            case .failed: ice = "失敗"
            case .closed: ice = "終了"
            case .count: ice = "不明"
            @unknown default: ice = "不明"
            }
            let snapshot: CallPeerDiagnostics = await withCheckedContinuation { continuation in
                peer.connection.statistics { report in
                    var received: UInt64 = 0
                    var sent: UInt64 = 0
                    var route = "未確立"
                    for stat in report.statistics.values {
                        if stat.type == "inbound-rtp", stat.values["kind"] as? String == "audio" {
                            received += (stat.values["bytesReceived"] as? NSNumber)?.uint64Value ?? 0
                        }
                        if stat.type == "outbound-rtp", stat.values["kind"] as? String == "audio" {
                            sent += (stat.values["bytesSent"] as? NSNumber)?.uint64Value ?? 0
                        }
                        if stat.type == "transport", let pairID = stat.values["selectedCandidatePairId"] as? String,
                           let pair = report.statistics[pairID] {
                            let kinds = ["localCandidateId", "remoteCandidateId"].compactMap { key -> String? in
                                guard let candidateID = pair.values[key] as? String else { return nil }
                                return report.statistics[candidateID]?.values["candidateType"] as? String
                            }
                            if kinds.contains("relay") { route = "中継（TURN）" }
                            else if !kinds.isEmpty { route = "直接接続" }
                        }
                    }
                    continuation.resume(returning: CallPeerDiagnostics(id: id, state: state, iceState: ice,
                        route: route, receivedBytes: received, sentBytes: sent))
                }
            }
            guard !Task.isCancelled, generation == epoch else { return [] }
            if peers[id] === peer { results.append(snapshot) }
        }
        return results.filter { peers[$0.id] === observedPeers[$0.id] }
    }

    private func description(_ connection: RTCPeerConnection, offer: Bool) async throws -> String {
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            let completion: @Sendable (RTCSessionDescription?, Error?) -> Void = { description, error in
                if let error { continuation.resume(throwing: error) }
                else if let description { continuation.resume(returning: description.sdp) }
                else { continuation.resume(throwing: ZooError.invalidResponse) }
            }
            if offer { connection.offer(for: constraints, completionHandler: completion) }
            else { connection.answer(for: constraints, completionHandler: completion) }
        }
    }

    private func setDescription(_ connection: RTCPeerConnection, sdp: String, type: RTCSdpType, local: Bool) async throws {
        try Task.checkCancellation()
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            let completion: @Sendable (Error?) -> Void = { error in
                if let error { continuation.resume(throwing: error) } else { continuation.resume() }
            }
            let description = RTCSessionDescription(type: type, sdp: sdp)
            if local { connection.setLocalDescription(description, completionHandler: completion) }
            else { connection.setRemoteDescription(description, completionHandler: completion) }
        }
    }

    private func add(_ value: CallSignal.Candidate, to connection: RTCPeerConnection) async throws {
        try Task.checkCancellation()
        let candidate = RTCIceCandidate(sdp: value.candidate, sdpMLineIndex: value.sdpMLineIndex, sdpMid: value.sdpMid)
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            connection.add(candidate) { error in
                if let error { continuation.resume(throwing: error) } else { continuation.resume() }
            }
        }
    }

    private func flush(_ peer: Peer) async throws {
        let candidates = peer.candidates
        peer.candidates = []
        for value in candidates { try await add(value, to: peer.connection) }
    }

    func mute(_ muted: Bool) { track?.isEnabled = !muted }

    func speaker(_ enabled: Bool) throws {
        let audio = RTCAudioSession.sharedInstance()
        audio.lockForConfiguration()
        defer { audio.unlockForConfiguration() }
        try audio.overrideOutputAudioPort(enabled ? .speaker : .none)
    }

    func close() {
        generation = UUID()
        systemAudioActive = false
        waitingAudio.stop()
        if let audioObserver { RTCAudioSession.sharedInstance().remove(audioObserver) }
        audioObserver = nil
        webRTCAudioRunning = false
        mediaOperation?.cancel(); mediaOperation = nil
        signal = nil; failed = nil; onPeerState = nil
        track?.isEnabled = false
        peers.values.forEach { $0.recovery?.cancel(); $0.connection.close() }
        peers = [:]; early = [:]; left = []; track = nil
        if audioActive {
            audioActive = false
            let audio = RTCAudioSession.sharedInstance()
            audio.lockForConfiguration()
            defer { audio.unlockForConfiguration() }
            try? audio.setActive(false)
        }
    }
}

extension NativeCallMedia: RTCPeerConnectionDelegate {
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {
        let identity = ObjectIdentifier(peerConnection)
        let value = CallSignal.Candidate(candidate: candidate.sdp, sdpMid: candidate.sdpMid, sdpMLineIndex: candidate.sdpMLineIndex)
        Task { @MainActor [weak self] in
            guard let self, let id = peers.first(where: { ObjectIdentifier($0.value.connection) == identity })?.key else { return }
            guard let peer = peers[id] else { return }
            if peer.sendingDescription {
                guard peer.localCandidates.count < 128 else { failed?(); return }
                peer.localCandidates.append(value)
            } else { signal?(id, CallSignal(type: "candidate", candidate: value)) }
        }
    }
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCPeerConnectionState) {
        let identity = ObjectIdentifier(peerConnection)
        Task { @MainActor [weak self] in
            guard let self, let peer = peers.values.first(where: { ObjectIdentifier($0.connection) == identity }) else { return }
            connectionChanged(peer.connection)
        }
    }
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}
    nonisolated func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {}
}
