import AVFAudio
import Foundation
@preconcurrency import WebRTC

@MainActor
protocol CallMedia: AnyObject {
    func start(sessionID: String, room: CallRoom, signal: @escaping @MainActor (String, CallSignal) -> Void,
               failed: @escaping @MainActor () -> Void) throws
    func handle(_ event: CallEvent) async throws
    func mute(_ muted: Bool)
    func speaker(_ enabled: Bool) throws
    func close()
}

@MainActor
final class NativeCallMedia: NSObject, CallMedia {
    private final class Peer {
        let connection: RTCPeerConnection
        var candidates: [CallSignal.Candidate] = []
        var lastAnswer: (offer: String, answer: String)?
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
    private var mediaOperation: Task<Void, Error>?
    private var signal: (@MainActor (String, CallSignal) -> Void)?
    private var failed: (@MainActor () -> Void)?

    func start(sessionID: String, room: CallRoom, signal: @escaping @MainActor (String, CallSignal) -> Void,
               failed: @escaping @MainActor () -> Void) throws {
        self.sessionID = sessionID
        self.signal = signal
        self.failed = failed
        generation = UUID()
        iceServers = room.ice_servers.map { RTCIceServer(urlStrings: $0.urls, username: $0.username, credential: $0.credential) }
        let audio = RTCAudioSession.sharedInstance()
        audio.lockForConfiguration()
        defer { audio.unlockForConfiguration() }
        let configuration = RTCAudioSessionConfiguration.webRTC()
        configuration.category = AVAudioSession.Category.playAndRecord.rawValue
        configuration.mode = AVAudioSession.Mode.voiceChat.rawValue
        configuration.categoryOptions = [.allowBluetoothHFP]
        try audio.setConfiguration(configuration, active: true)
        audioActive = true
        track = factory.audioTrack(withTrackId: "zoo-audio")
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
        return peer
    }

    private func join(_ id: String) async throws {
        guard id != sessionID, !left.contains(id), peers[id] == nil, sessionID < id else { return }
        let epoch = generation
        let peer = try peer(id)
        let sdp = try await description(peer.connection, offer: true)
        try await setDescription(peer.connection, sdp: sdp, type: .offer, local: true)
        guard generation == epoch else { return }
        signal?(id, CallSignal(type: "offer", sdp: sdp))
    }

    func handle(_ event: CallEvent) async throws {
        let previous = mediaOperation
        let epoch = generation
        // SSEと送信失敗の双方から退出が来ても、実行中のSDP操作にcloseを割り込ませない。
        let operation = Task { @MainActor [weak self] in
            _ = try? await previous?.value
            guard let self, generation == epoch else { return }
            try Task.checkCancellation()
            try await perform(event)
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
            peers.removeValue(forKey: id)?.connection.close()
            early[id] = nil
        case .signal(let id, let value):
            guard id != sessionID, !left.contains(id), UUID(uuidString: id) != nil else { return }
            switch value.type {
            case "offer":
                guard let sdp = value.sdp else { throw ZooError.invalidResponse }
                let peer = try peer(id)
                if let cached = peer.lastAnswer, cached.offer == sdp {
                    signal?(id, CallSignal(type: "answer", sdp: cached.answer)); return
                }
                try await setDescription(peer.connection, sdp: sdp, type: .offer, local: false)
                try await flush(peer)
                let answer = try await description(peer.connection, offer: false)
                try await setDescription(peer.connection, sdp: answer, type: .answer, local: true)
                guard epoch == generation else { return }
                peer.lastAnswer = (sdp, answer)
                signal?(id, CallSignal(type: "answer", sdp: answer))
            case "answer":
                guard let sdp = value.sdp, let peer = peers[id], peer.connection.signalingState == .haveLocalOffer else { return }
                try await setDescription(peer.connection, sdp: sdp, type: .answer, local: false)
                try await flush(peer)
            case "candidate":
                guard let candidate = value.candidate else { return }
                if let peer = peers[id] {
                    if peer.connection.remoteDescription != nil { try await add(candidate, to: peer.connection) }
                    else if peer.candidates.count < 128 { peer.candidates.append(candidate) }
                } else if early.count < 6, (early[id]?.count ?? 0) < 128 {
                    early[id, default: []].append(candidate)
                }
            default: break
            }
        }
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
        mediaOperation?.cancel(); mediaOperation = nil
        signal = nil; failed = nil
        track?.isEnabled = false
        peers.values.forEach { $0.connection.close() }
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
            signal?(id, CallSignal(type: "candidate", candidate: value))
        }
    }
    nonisolated func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCPeerConnectionState) {
        guard newState == .failed else { return }
        let identity = ObjectIdentifier(peerConnection)
        Task { @MainActor [weak self] in
            guard let self, peers.values.contains(where: { ObjectIdentifier($0.connection) == identity }) else { return }
            failed?()
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
