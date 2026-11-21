import Foundation
import Testing
@testable import Zoo

@MainActor @Suite("Apple Watchの接続と通話操作")
struct WatchCompanionTests {
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    private func snapshot(phase: WatchCall.Phase = .incoming) -> WatchSnapshot {
        WatchSnapshot(sessionID: UUID(), call: WatchCall(id: UUID(), title: "相手", phase: phase,
            status: "着信中", muted: false, canSetMuted: phase == .connected, expiresAt: now.addingTimeInterval(30)))
    }

    private func connected(_ snapshot: WatchSnapshot) throws -> WatchCompanionState {
        let state = WatchCompanionState()
        state.setActive(true)
        state.setReachable(true)
        let request = try #require(state.begin(.refresh))
        state.complete(request, reply: WatchReply(createdAt: now, snapshot: snapshot))
        return state
    }

    @Test("切断中と非表示中は操作を送信せず、保持していた通知と着信を消す", arguments: [false, true])
    func disconnect(inactive: Bool) throws {
        let state = try connected(snapshot())
        let request = try #require(state.begin(.answer, at: now))
        if inactive { state.setActive(false) } else { state.setReachable(false) }
        #expect(state.snapshot == nil && state.pending == nil)
        #expect(state.begin(.answer, at: now) == nil)
        state.complete(request, reply: WatchReply(createdAt: now, snapshot: snapshot()))
        #expect(state.snapshot == nil)
    }

    @Test("再接続時は新しい要求に対応する応答だけを採用する")
    func ignoresOldReplyAfterReconnect() throws {
        let state = try connected(snapshot())
        let old = try #require(state.begin(.refresh))
        state.setReachable(false)
        state.setReachable(true)
        let fresh = try #require(state.begin(.refresh))
        state.receive(WatchReply(createdAt: now, snapshot: snapshot()))
        state.complete(old, reply: WatchReply(createdAt: now, snapshot: snapshot()))
        #expect(state.snapshot == nil && state.pending == fresh.id)
        state.complete(fresh, reply: WatchReply(createdAt: now, snapshot: WatchSnapshot()))
        #expect(state.snapshot?.sessionID == nil && state.pending == nil)
    }

    @Test("期限切れの着信には応答・拒否を送信しない", arguments: [WatchAction.answer, .decline])
    func rejectsExpired(action: WatchAction) throws {
        let state = try connected(snapshot())
        #expect(state.begin(action, at: now.addingTimeInterval(30)) == nil)
        #expect(state.pending == nil)
    }

    @Test("通話IDと接続セッションを指定し、連打中の重複送信を防ぐ")
    func serializesActions() throws {
        let snapshot = snapshot(phase: .connected)
        let state = try connected(snapshot)
        let request = try #require(state.begin(.setMuted, muted: true))
        #expect(request.callID == snapshot.call?.id)
        #expect(request.sessionID == snapshot.sessionID && request.muted == true)
        #expect(state.begin(.setMuted, muted: false) == nil)
        state.complete(request, reply: WatchReply(createdAt: now, snapshot: snapshot, error: "操作できません"))
        #expect(state.error == "操作できません" && state.notice == nil && state.pending == nil)
    }

    @Test("通話終了の更新後に古い操作応答が届いても着信を復元しない")
    func preservesNewerState() throws {
        let old = snapshot()
        let state = try connected(old)
        let request = try #require(state.begin(.answer, at: now))
        let ended = WatchSnapshot(sessionID: old.sessionID)
        state.receive(WatchReply(createdAt: now.addingTimeInterval(2), snapshot: ended))
        state.complete(request, reply: WatchReply(createdAt: now.addingTimeInterval(1), snapshot: old))
        #expect(state.snapshot?.call == nil && state.pending == nil)
    }

    @Test("通信失敗時は古い操作対象を破棄し、再読み込みを要求する")
    func failedCommand() throws {
        let state = try connected(snapshot())
        let request = try #require(state.begin(.answer, at: now))
        state.fail(request)
        #expect(state.snapshot == nil && state.error != nil && state.pending == nil)
        #expect(state.begin(.answer, at: now) == nil)
        #expect(state.begin(.refresh) != nil)
    }

    @Test("未知の通信バージョンでは通話操作を有効にしない")
    func unsupportedVersion() throws {
        let state = try connected(snapshot())
        let request = try #require(state.begin(.refresh))
        state.complete(request, reply: WatchReply(version: 2, createdAt: now, snapshot: snapshot()))
        #expect(state.snapshot == nil && state.error != nil)
    }

    @Test("接続中は終了だけを許可し、ミュートや再応答を拒否する")
    func connectingActions() throws {
        let call = try #require(snapshot(phase: .connecting).call)
        #expect(call.allows(.end, at: now))
        #expect(!call.allows(.answer, at: now) && !call.allows(.decline, at: now))
        #expect(!call.allows(.setMuted, at: now))
    }

    @Test("Watchとの通信で通知と通話IDを欠落させない")
    func roundTrip() throws {
        var snapshot = snapshot()
        snapshot.notices = [WatchNotice(id: "notice", title: "返信", detail: "相手", date: now, unread: true)]
        let data = try JSONEncoder().encode(WatchReply(createdAt: now, snapshot: snapshot))
        let reply = try JSONDecoder().decode(WatchReply.self, from: data)
        #expect(reply.snapshot == snapshot)
        let request = WatchRequest(action: .answer, sessionID: snapshot.sessionID, callID: snapshot.call?.id)
        let decoded = try JSONDecoder().decode(WatchRequest.self, from: JSONEncoder().encode(request))
        #expect(decoded.id == request.id && decoded.callID == request.callID && decoded.sessionID == request.sessionID)
    }

    @Test("表示の送信失敗は最新状態で2回まで再試行する")
    func retriesLatestSnapshot() async throws {
        let probe = DeliveryProbe(current: snapshot())
        let publisher = probe.publisher()
        var sent = probe.sent.stream.makeAsyncIterator()
        var waiting = probe.waiting.stream.makeAsyncIterator()
        publisher.publish()
        _ = await sent.next()
        for index in 0..<2 {
            probe.failures[index]()
            _ = await waiting.next()
            probe.current = snapshot(phase: .connected)
            probe.resume.continuation.yield(())
            _ = await sent.next()
            #expect(probe.snapshots.last == probe.current)
        }
        probe.failures[2]()
        #expect(probe.waitCount == 2 && probe.snapshots.count == 3)
    }

    @Test("古い送信の失敗は新しい成功済み表示を無効化しない")
    func ignoresSupersededFailure() {
        let probe = DeliveryProbe(current: snapshot())
        let publisher = probe.publisher()
        publisher.publish()
        probe.current = snapshot(phase: .connected)
        publisher.publish()
        probe.failures[0]()
        publisher.publish()
        #expect(probe.snapshots.count == 2)
    }

    @Test("再試行を待つ間に切断したら古い表示を送信しない")
    func stopsRetryWhenDisconnected() async {
        let probe = DeliveryProbe(current: snapshot())
        let publisher = probe.publisher()
        var reads = probe.reads.stream.makeAsyncIterator()
        var waiting = probe.waiting.stream.makeAsyncIterator()
        publisher.publish()
        _ = await reads.next()
        probe.failures[0]()
        _ = await waiting.next()
        probe.current = nil
        probe.resume.continuation.yield(())
        _ = await reads.next()
        #expect(probe.snapshots.count == 1)
    }

    @MainActor private final class DeliveryProbe {
        var current: WatchSnapshot?
        var snapshots: [WatchSnapshot] = []
        var failures: [@MainActor @Sendable () -> Void] = []
        var waitCount = 0
        let reads = AsyncStream<Void>.makeStream()
        let sent = AsyncStream<Void>.makeStream()
        let waiting = AsyncStream<Void>.makeStream()
        let resume = AsyncStream<Void>.makeStream()

        init(current: WatchSnapshot) { self.current = current }

        func publisher() -> WatchSnapshotPublisher {
            WatchSnapshotPublisher(snapshot: { [self] in
                reads.continuation.yield(())
                return current
            }, send: { [self] data, failed in
                if let reply = try? JSONDecoder().decode(WatchReply.self, from: data) { snapshots.append(reply.snapshot) }
                failures.append(failed)
                sent.continuation.yield(())
            }, pause: { [self] in
                waitCount += 1
                waiting.continuation.yield(())
                for await _ in resume.stream { return }
            })
        }
    }
}
