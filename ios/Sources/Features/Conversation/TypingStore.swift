import Foundation
import Observation

@Observable @MainActor
final class TypingStore {
    private(set) var users: [TypingUser] = []
    private(set) var parentID: String?
    @ObservationIgnored var onExpiredSession: (() -> Void)?
    private let client: any TypingClient
    private let channelID: String
    private let userID: String
    private let pollInterval: Duration
    private let idleTimeout: Duration
    private let sendInterval: Duration
    private var generation = UUID()
    private var session: TypingSession?

    init(client: any TypingClient, channelID: String, userID: String,
         pollInterval: Duration = .milliseconds(1500), idleTimeout: Duration = .seconds(3),
         sendInterval: Duration = .seconds(2)) {
        self.client = client
        self.channelID = channelID
        self.userID = userID
        self.pollInterval = pollInterval
        self.idleTimeout = idleTimeout
        self.sendInterval = sendInterval
    }

    func indicator(parentID: String?) -> String? {
        guard session != nil, self.parentID == parentID, !users.isEmpty else { return nil }
        let names = users.prefix(2).map { $0.display_name.isEmpty ? "ユーザー" : $0.display_name }.joined(separator: "、")
        let others = users.count > 2 ? "、ほか\(users.count - 2)人" : ""
        return "\(names)\(others)が入力中…"
    }

    func activity(_ hasText: Bool, parentID: String?) {
        guard self.parentID == parentID else { return }
        session?.activity(hasText)
    }

    func stop() {
        generation = UUID()
        session?.finish()
        session = nil
        users = []
        parentID = nil
    }

    func run(parentID: String?) async {
        guard !Task.isCancelled else { return }
        stop()
        let epoch = generation
        self.parentID = parentID
        session = TypingSession(client: client, channelID: channelID, parentID: parentID,
                                idleTimeout: idleTimeout, sendInterval: sendInterval)
        defer { if generation == epoch { stop() } }
        var delay = pollInterval
        while !Task.isCancelled, generation == epoch {
            do {
                let snapshot = try await client.typingUsers(channelID: channelID, parentID: parentID)
                guard !Task.isCancelled, generation == epoch else { return }
                var seen = Set<String>()
                users = snapshot.filter { $0.id != userID && seen.insert($0.id).inserted }.sorted { $0.id < $1.id }
                delay = users.isEmpty && session?.active != true ? min(delay * 2, .seconds(5)) : pollInterval
            } catch {
                guard !Task.isCancelled, generation == epoch else { return }
                users = []
                if case ZooError.expiredSession = error { onExpiredSession?(); return }
                if case ZooError.rejected(let status, _) = error, status == 403 || status == 404 { return }
                delay = min(delay * 2, .seconds(5))
            }
            do { try await Task.sleep(for: delay) } catch { return }
        }
    }
}

@MainActor
private final class TypingSession {
    private let client: any TypingClient
    private let channelID: String
    private let parentID: String?
    private let id = UUID()
    private let idleTimeout: Duration
    private let sendInterval: Duration
    private(set) var active = false
    private var ended = false
    private var publishedActive = false
    private var activityRevision = 0
    private var lastSent: ContinuousClock.Instant?
    private var writer: Task<Void, Never>?
    private var idle: Task<Void, Never>?

    init(client: any TypingClient, channelID: String, parentID: String?, idleTimeout: Duration, sendInterval: Duration) {
        self.client = client
        self.channelID = channelID
        self.parentID = parentID
        self.idleTimeout = idleTimeout
        self.sendInterval = sendInterval
    }

    func activity(_ hasText: Bool) {
        guard !ended else { return }
        idle?.cancel()
        idle = nil
        active = hasText
        activityRevision += 1
        publish()
        if hasText {
            let timeout = idleTimeout
            idle = Task { [weak self] in
                do { try await Task.sleep(for: timeout) } catch { return }
                guard !Task.isCancelled else { return }
                self?.activity(false)
            }
        }
    }

    func finish() {
        activity(false)
        ended = true
    }

    private func publish() {
        guard writer == nil else { return }
        writer = Task {
            defer { writer = nil }
            while active || publishedActive {
                let value = active
                let revision = activityRevision
                if value, let lastSent, lastSent.duration(to: .now) < sendInterval { return }
                // 応答が失われても開始が届いた可能性があるため、後続の停止は必ず送る。
                publishedActive = value
                lastSent = value ? .now : nil
                do { try await client.setTyping(channelID: channelID, parentID: parentID, sessionID: id, active: value) }
                catch { /* 入力中通知の失敗で、本文の入力や送信を妨げない。 */ }
                // 同時に要求せず、通信中に変わった最新の状態だけを続けて送る。
                if active == value && activityRevision == revision { return }
            }
        }
    }
}
