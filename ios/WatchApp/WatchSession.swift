import Foundation
import WatchConnectivity

@MainActor
final class WatchSession: NSObject {
    let state = WatchCompanionState()
    private let connectivity = WCSession.default

    override init() {
        super.init()
        connectivity.delegate = self
        connectivity.activate()
    }

    func setActive(_ active: Bool) {
        state.setActive(active)
        updateReachability()
        if active { send(.refresh) }
    }

    private func updateReachability() {
        state.setReachable(connectivity.activationState == .activated && connectivity.isReachable)
    }

    func send(_ action: WatchAction, muted: Bool? = nil) {
        updateReachability()
        guard let request = state.begin(action, muted: muted) else { return }
        guard let data = try? JSONEncoder().encode(request) else { state.fail(request); return }
        connectivity.sendMessageData(data) { @Sendable [weak self] data in
            let reply = try? JSONDecoder().decode(WatchReply.self, from: data)
            Task { @MainActor in
                if let reply { self?.state.complete(request, reply: reply) }
                else { self?.state.fail(request) }
            }
        } errorHandler: { @Sendable [weak self] _ in
            Task { @MainActor in self?.state.fail(request) }
        }
    }
}

extension WatchSession: WCSessionDelegate {
    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState,
                             error: (any Error)?) {
        Task { @MainActor [weak self] in
            self?.updateReachability()
            self?.send(.refresh)
        }
    }

    nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
        Task { @MainActor [weak self] in
            self?.updateReachability()
            self?.send(.refresh)
        }
    }

    nonisolated func session(_ session: WCSession, didReceiveMessageData messageData: Data) {
        guard let reply = try? JSONDecoder().decode(WatchReply.self, from: messageData) else { return }
        Task { @MainActor [weak self] in self?.state.receive(reply) }
    }
}
