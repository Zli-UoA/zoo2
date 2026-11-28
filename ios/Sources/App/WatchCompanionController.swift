import Foundation
import Observation
import WatchConnectivity

// SDKの返信ブロックは任意キューで呼べるがSendable未指定。単一Taskだけへ渡して一度呼ぶ。
// SDKにSendableが付いたらこの受け渡し用の型を外す。
private struct WatchReplyHandler: @unchecked Sendable {
    let send: (Data) -> Void
}

@MainActor
final class WatchSnapshotPublisher {
    private let snapshot: () -> WatchSnapshot?
    private let send: (Data, @escaping @MainActor @Sendable () -> Void) -> Void
    private let pause: () async throws -> Void
    private var lastSent: WatchSnapshot?
    private var generation = UUID()
    private var retryTask: Task<Void, Never>?

    init(snapshot: @escaping () -> WatchSnapshot?,
         send: @escaping (Data, @escaping @MainActor @Sendable () -> Void) -> Void,
         pause: @escaping () async throws -> Void = { try await Task.sleep(for: .seconds(1)) }) {
        self.snapshot = snapshot
        self.send = send
        self.pause = pause
    }

    deinit { retryTask?.cancel() }

    func publish(force: Bool = false) { publish(force: force, retries: 2) }

    private func publish(force: Bool, retries: Int) {
        guard let snapshot = snapshot() else {
            retryTask?.cancel()
            retryTask = nil
            generation = UUID()
            lastSent = nil
            return
        }
        guard force || snapshot != lastSent,
              let data = try? JSONEncoder().encode(WatchReply(snapshot: snapshot)) else { return }
        retryTask?.cancel()
        retryTask = nil
        let id = UUID()
        generation = id
        lastSent = snapshot
        send(data) { [weak self] in
            guard let self, self.generation == id else { return }
            self.lastSent = nil
            guard retries > 0 else { return }
            let pause = self.pause
            self.retryTask = Task { [weak self] in
                do { try await pause(); try Task.checkCancellation() } catch { return }
                guard let self, self.generation == id else { return }
                self.retryTask = nil
                // 古い着信やアカウントを再送せず、再試行時の状態と到達性を取り直す。
                self.publish(force: true, retries: retries - 1)
            }
        }
    }
}

@MainActor
final class WatchCompanionController: NSObject {
    private let service: WatchCompanionService
    private let calls: SystemCallController
    private let notifications: NotificationController
    private let incomingAlerts = IncomingCallNotifications()
    private let connectivity: WCSession
    private var observationID = UUID()
    private lazy var publisher = WatchSnapshotPublisher(snapshot: { [weak self] in
        guard let self, self.connectivity.activationState == .activated, self.connectivity.isPaired,
              self.connectivity.isWatchAppInstalled, self.connectivity.isReachable else { return nil }
        return self.service.snapshot()
    }, send: { [weak self] data, failed in
        // 通話操作と個人情報をバックグラウンド配送のキューへ残さない。
        self?.connectivity.sendMessageData(data, replyHandler: nil) { @Sendable _ in
            Task { @MainActor in failed() }
        }
    })

    init?(session: AppSession, calls: SystemCallController) {
        guard WCSession.isSupported() else { return nil }
        service = WatchCompanionService(session: session, calls: calls)
        self.calls = calls
        notifications = session.notifications
        connectivity = .default
        super.init()
    }

    func start() {
        Task { await incomingAlerts.removePreviousNotifications() }
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
            _ = notifications.enabled
        } onChange: { [weak self] in
            Task { @MainActor in
                guard self?.observationID == id else { return }
                self?.observe()
            }
        }
        publish()
        // Watchアプリの到達性に依存せず、閉じている間もOSの通知転送を使う。
        incomingAlerts.update(calls.incomingNotification, enabled: notifications.enabled &&
            connectivity.activationState == .activated && connectivity.isPaired)
    }

    private func publish(force: Bool = false) {
        publisher.publish(force: force)
    }
}

extension WatchCompanionController: WCSessionDelegate {
    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState,
                             error: (any Error)?) {
        Task { @MainActor [weak self] in self?.observe(); self?.publish(force: true) }
    }

    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}

    nonisolated func sessionWatchStateDidChange(_ session: WCSession) {
        Task { @MainActor [weak self] in self?.observe() }
    }

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
