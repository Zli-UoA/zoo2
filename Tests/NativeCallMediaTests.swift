import Foundation
import Testing
@testable import Zoo

// RTCAudioSessionはプロセス共通なので、実WebRTCを使う試験同士だけを直列にする。
@MainActor @Suite("実WebRTCの通話接続", .serialized, .timeLimit(.minutes(1)))
struct NativeCallMediaTests {
    @MainActor private final class Pair {
        let firstID = "11111111-1111-4111-8111-111111111111"
        let secondID = "22222222-2222-4222-8222-222222222222"
        let first: NativeCallMedia
        let second: NativeCallMedia
        let signals = AsyncStream<(Bool, CallSignal)>.makeStream()
        var delivery: Task<Void, Never>?
        var states: [Bool: CallPeerState] = [:]
        var failure = false
        var offers = 0
        var signalTypes: [String] = []
        var offerSDPs: [String] = []
        var releaseCandidates: Bool
        let simultaneous: Bool
        var restartSenders: Set<Bool> = []
        var heldSignals: [(Bool, CallSignal)] = []
        var releasedCollision = false

        init(restart: Bool, simultaneous: Bool = false) {
            first = NativeCallMedia(connectionTimeout: .seconds(restart ? 2 : 10), restartTimeout: .seconds(3))
            second = NativeCallMedia(connectionTimeout: .seconds(simultaneous ? 2 : 10), restartTimeout: .seconds(3))
            releaseCandidates = !restart
            self.simultaneous = simultaneous
            first.systemAudio = true
            second.systemAudio = true
        }

        func start() async throws {
            let participants = [firstID, secondID].map {
                CallParticipant(session_id: $0, user_id: $0, user_name: "テスト", display_name: "テスト", is_muted: false)
            }
            let room = CallRoom(participants: participants, max_participants: 6, ice_servers: [])
            first.onPeerState = { [weak self] _, state in self?.states[true] = state }
            second.onPeerState = { [weak self] _, state in self?.states[false] = state }
            try first.start(sessionID: firstID, room: room, signal: { [weak self] _, signal in
                self?.signals.continuation.yield((true, signal))
            }, failed: { [weak self] in self?.failure = true })
            try second.start(sessionID: secondID, room: room, signal: { [weak self] _, signal in
                self?.signals.continuation.yield((false, signal))
            }, failed: { [weak self] in self?.failure = true })
            delivery = Task { [weak self] in
                guard let self else { return }
                for await (fromFirst, signal) in signals.stream {
                    do {
                        if fromFirst { signalTypes.append(signal.type) }
                        if fromFirst, signal.type == "offer" {
                            offers += 1
                            offerSDPs.append(signal.sdp ?? "")
                            if offers >= 2 { releaseCandidates = true }
                        }
                        if simultaneous, !releasedCollision {
                            if signal.type == "offer", !fromFirst || offers >= 2 { restartSenders.insert(fromFirst) }
                            if !restartSenders.isEmpty {
                                heldSignals.append((fromFirst, signal))
                                guard restartSenders.count == 2 else { continue }
                                // 両側でsetLocalDescriptionが済むまで配送せず、衝突経路を必ず通す。
                                releasedCollision = true
                                let held = heldSignals
                                heldSignals = []
                                for (sender, pending) in held { try await deliver(sender, pending) }
                                continue
                            }
                        }
                        try await deliver(fromFirst, signal)
                    } catch {
                        if !Task.isCancelled { failure = true; Issue.record(error) }
                        return
                    }
                }
            }
            try await first.handle(.room(room))
            try await second.handle(.room(room))
        }

        private func deliver(_ fromFirst: Bool, _ signal: CallSignal) async throws {
            if signal.type == "candidate", !releaseCandidates { return }
            try await (fromFirst ? second : first).handle(.signal(fromFirst ? firstID : secondID, signal))
        }

        func waitForConnection() async throws {
            let deadline = ContinuousClock.now.advanced(by: .seconds(12))
            while !failure, states.values.filter({ $0 == .connected }).count < 2, ContinuousClock.now < deadline {
                try await Task.sleep(for: .milliseconds(20))
            }
            #expect(!failure)
            #expect(states[true] == .connected && states[false] == .connected)
        }

        func close() {
            delivery?.cancel()
            signals.continuation.finish()
            first.close(); second.close()
        }
    }

    @Test("実PeerConnection同士が接続し、接続状態と経路を診断で確認できる")
    func connect() async throws {
        let pair = Pair(restart: false)
        defer { pair.close() }
        try await pair.start()
        try await pair.waitForConnection()
        #expect(pair.signalTypes.first == "offer")
        let diagnostics = await pair.first.diagnostics()
        #expect(diagnostics.count == 1)
        #expect(diagnostics.first?.state == .connected)
        #expect(diagnostics.first?.route == "直接接続")
        #expect(diagnostics.first?.iceState == "接続済み")
    }

    @Test("最初のICE候補を失っても再交渉し、候補より先に新offerを送る")
    func restartAfterLostCandidates() async throws {
        let pair = Pair(restart: true)
        defer { pair.close() }
        try await pair.start()
        try await pair.waitForConnection()
        #expect(pair.offers >= 2)
        let sdps = pair.offerSDPs.compactMap { sdp in sdp.components(separatedBy: "\n").first { $0.hasPrefix("a=ice-ufrag:") } }
        #expect(Set(sdps).count >= 2)
    }

    @Test("両端が同時にICE再接続してもoffer衝突から復帰する")
    func simultaneousRestart() async throws {
        let pair = Pair(restart: true, simultaneous: true)
        defer { pair.close() }
        try await pair.start()
        try await pair.waitForConnection()
        #expect(pair.restartSenders.count == 2 && pair.releasedCollision)
    }

    @Test("相手が応答しない場合は接続期限で失敗し、終了時に診断を破棄する")
    func unansweredPeerTimesOut() async throws {
        let media = NativeCallMedia(connectionTimeout: .milliseconds(30))
        media.systemAudio = true
        defer { media.close() }
        let remote = CallParticipant(session_id: "22222222-2222-4222-8222-222222222222", user_id: "other",
                                     user_name: "相手", display_name: "相手", is_muted: false)
        let room = CallRoom(participants: [remote], max_participants: 6, ice_servers: [])
        var failed = false
        try media.start(sessionID: "11111111-1111-4111-8111-111111111111", room: room, signal: { _, _ in }, failed: { failed = true })
        try await media.handle(.room(room))
        let deadline = ContinuousClock.now.advanced(by: .seconds(3))
        while !failed, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(10)) }
        #expect(failed)
        media.close()
        #expect(await media.diagnostics().isEmpty)
    }
}
