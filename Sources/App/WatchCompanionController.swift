import Foundation
import Observation
import WatchConnectivity

// SDKの返信ブロックは任意キューで呼べるがSendable未指定。単一Taskだけへ渡して一度呼ぶ。
// SDKにSendableが付いたらこの受け渡し用の型を外す。
private struct WatchReplyHandler: @unchecked Sendable {
    let send: (Data) -> Void
}

@MainActor
final class WatchCompanionController: NSObject {
    private let service: WatchCompanionService
    private let calls: SystemCallController
    private let connectivity: WCSession
    private var lastSent: WatchSnapshot?
    private var observationID = UUID()

    init?(session: AppSession, calls: SystemCallController) {
        guard WCSession.isSupported() else { return nil }
        service = WatchCompanionService(session: session, calls: calls)
        self.calls = calls
        connectivity = .default
        super.init()
    }

    func start() {
        connectivity.delegate = self
        connectivity.activate()
        calls.onWatchStateChanged = { [weak self] in self?.observe() }
        observe()
    }

    private func observe() {
        let id = UUID()
        observationID = id
        withObservationTracking {
            _ = service.snapshot()
        } onChange: { [weak self] in
            Task { @MainActor in
                guard self?.observationID == id else { return }
                self?.observe()
            }
        }
        publish()
    }

    private func publish(force: Bool = false) {
        guard connectivity.activationState == .activated, connectivity.isPaired,
              connectivity.isWatchAppInstalled, connectivity.isReachable else { return }
        let snapshot = service.snapshot()
        guard force || snapshot != lastSent, let data = try? JSONEncoder().encode(WatchReply(snapshot: snapshot)) else { return }
        lastSent = snapshot
        // 通話操作と個人情報をバックグラウンド配送のキューへ残さない。
        connectivity.sendMessageData(data, replyHandler: nil) { @Sendable [weak self] _ in
            Task { @MainActor in self?.lastSent = nil }
        }
    }
}

extension WatchCompanionController: WCSessionDelegate {
    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState,
                             error: (any Error)?) {
        Task { @MainActor [weak self] in self?.publish(force: true) }
    }

    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}

    nonisolated func sessionDidDeactivate(_ session: WCSession) {
        session.activate()
    }

    nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
        Task { @MainActor [weak self] in self?.publish(force: true) }
    }

    nonisolated func session(_ session: WCSession, didReceiveMessageData messageData: Data,
                             replyHandler: @escaping (Data) -> Void) {
        let request = try? JSONDecoder().decode(WatchRequest.self, from: messageData)
        let response = WatchReplyHandler(send: replyHandler)
        Task { @MainActor [weak self] in
            let reply: WatchReply
            if let self, let request {
                reply = await service.respond(to: request)
            } else {
                reply = WatchReply(snapshot: WatchSnapshot(), error: "要求を読み取れませんでした。Zooを更新してください。")
            }
            response.send((try? JSONEncoder().encode(reply)) ?? Data())
        }
    }
}
