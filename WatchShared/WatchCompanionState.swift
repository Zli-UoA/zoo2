import Foundation
import Observation

@Observable @MainActor
final class WatchCompanionState {
    private(set) var snapshot: WatchSnapshot?
    private(set) var reachable = false
    private(set) var active = false
    private(set) var pending: UUID?
    private(set) var error: String?
    private(set) var notice: String?
    private var receivedAt = Date.distantPast

    func setActive(_ value: Bool) {
        active = value
        if !value { clear() }
    }

    func setReachable(_ value: Bool) {
        reachable = value
        if !value { clear() }
    }

    private func clear() {
        // 再接続後に古いアカウントの表示や操作結果を復元しない。
        snapshot = nil
        pending = nil
        error = nil
        notice = nil
        receivedAt = .distantPast
    }

    func begin(_ action: WatchAction, muted: Bool? = nil, at date: Date = .now) -> WatchRequest? {
        guard active, reachable, pending == nil else { return nil }
        if action != .refresh {
            guard snapshot?.sessionID != nil, let call = snapshot?.call, call.allows(action, at: date),
                  action != .setMuted || muted != nil else { return nil }
        }
        let request = WatchRequest(action: action, sessionID: snapshot?.sessionID,
                                   callID: snapshot?.call?.id, muted: muted)
        pending = request.id
        error = nil
        notice = nil
        return request
    }

    func complete(_ request: WatchRequest, reply: WatchReply) {
        guard active, reachable, pending == request.id else { return }
        pending = nil
        guard reply.version == 1 else {
            snapshot = nil
            error = "iPhoneとWatchのZooを更新してください。"
            return
        }
        apply(reply)
        error = reply.error
        if request.action != .refresh, reply.error == nil {
            notice = "操作をiPhoneへ送信しました。"
        }
    }

    func fail(_ request: WatchRequest) {
        guard pending == request.id else { return }
        pending = nil
        snapshot = nil
        error = "iPhoneに接続できません。近くに置いてZooを開き、再読み込みしてください。"
    }

    func receive(_ reply: WatchReply) {
        // 再接続時は再読み込みの応答を待ち、前の接続から遅れて届いた表示を復元しない。
        guard active, reachable, snapshot != nil, reply.version == 1 else { return }
        apply(reply)
    }

    private func apply(_ reply: WatchReply) {
        guard reply.createdAt >= receivedAt else { return }
        receivedAt = reply.createdAt
        snapshot = reply.snapshot
    }
}
