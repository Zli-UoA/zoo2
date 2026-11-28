import Foundation
import Observation

@Observable @MainActor
final class ChannelNotifications {
    private let client: any ChatClient
    private let userID: String
    private var active = true
    private var levels: [String: ChannelNotificationLevel] = [:]
    private var receivedSnapshot = false
    private struct Saved {
        let level: ChannelNotificationLevel
        let transactionID: UInt64?
    }
    private var saved: [String: Saved] = [:]
    private var observedTransactions: Set<UInt64> = []
    private var snapshotGeneration = UUID()
    private var revision = UUID()
    private(set) var loading: Set<String> = []
    private(set) var saving: Set<String> = []
    private(set) var errors: [String: String] = [:]
    private(set) var syncError: String?
    var onExpiredSession: (() -> Void)?

    init(client: any ChatClient, userID: String) {
        self.client = client
        self.userID = userID
    }

    func level(for channelID: String) -> ChannelNotificationLevel? {
        saved[channelID]?.level ?? shapeLevel(for: channelID)
    }

    private func shapeLevel(for channelID: String) -> ChannelNotificationLevel? {
        levels[channelID] ?? (receivedSnapshot ? .mentions : nil)
    }

    func load(_ channelID: String) async {
        guard active, !saving.contains(channelID), loading.insert(channelID).inserted else { return }
        let revision = revision
        defer { loading.remove(channelID) }
        do {
            let result = try await client.channelNotificationSettings(id: channelID)
            try Task.checkCancellation()
            guard active, self.revision == revision else { return }
            levels[channelID] = result.level
            saved[channelID] = nil
            errors[channelID] = nil
        } catch { report(error, channelID: channelID) }
    }

    func save(_ level: ChannelNotificationLevel, channelID: String) async {
        guard active, !loading.contains(channelID), saving.insert(channelID).inserted else { return }
        revision = UUID()
        let revision = revision
        let snapshotGeneration = snapshotGeneration
        errors[channelID] = nil
        defer {
            saving.remove(channelID)
            if saving.isEmpty { observedTransactions = [] }
        }
        do {
            let result = try await client.setChannelNotificationSettings(id: channelID, level: level)
            guard active else { return }
            let transactionID = result.txid.flatMap(UInt64.init).map { UInt64(UInt32(truncatingIfNeeded: $0)) }
            // 中間の値がバッチ内で消えても、書き込みのtxidで同期完了を判断する。
            if self.snapshotGeneration != snapshotGeneration || transactionID.map({ observedTransactions.contains($0) }) == true ||
                (transactionID == nil && self.revision != revision) {
                saved[channelID] = nil
            } else {
                saved[channelID] = Saved(level: result.data.level, transactionID: transactionID)
            }
        } catch { report(error, channelID: channelID) }
    }

    func receive(_ rows: [ChannelNotificationSetting], transactionIDs: Set<UInt64> = [], freshSnapshot: Bool = false) {
        guard active else { return }
        revision = UUID()
        receivedSnapshot = true
        levels = Dictionary(rows.filter { $0.user_id == userID }.map { ($0.channel_id, $0.notification_level) },
                            uniquingKeysWith: { _, new in new })
        if !saving.isEmpty { observedTransactions.formUnion(transactionIDs) }
        if freshSnapshot {
            snapshotGeneration = UUID()
            saved = [:]
        } else {
            for (id, value) in saved where value.transactionID.map({ transactionIDs.contains($0) }) ?? true {
                saved[id] = nil
            }
        }
    }

    func run() async {
        var delay = 2
        while active, !Task.isCancelled {
            do {
                try await client.observeChannelNotificationSettings { [weak self] snapshot in
                    guard let self, let rows = snapshot.rows else { return }
                    self.receive(rows, transactionIDs: snapshot.transactionIDs, freshSnapshot: snapshot.freshSnapshot)
                    self.syncError = nil
                    delay = 2
                }
                try Task.checkCancellation()
                throw ZooError.invalidResponse
            } catch {
                guard active, !Task.isCancelled else { return }
                if case ZooError.expiredSession = error { onExpiredSession?(); return }
                syncError = "通知設定の同期に失敗しました。再接続しています。"
                do { try await Task.sleep(for: .seconds(delay)) } catch { return }
                delay = min(delay * 2, 30)
            }
        }
    }

    private func report(_ error: any Error, channelID: String) {
        guard active, !Task.isCancelled else { return }
        errors[channelID] = error.localizedDescription
        if case ZooError.expiredSession = error { onExpiredSession?() }
    }

    func shutdown() {
        active = false
        receivedSnapshot = false
        levels = [:]; saved = [:]; observedTransactions = []; errors = [:]; syncError = nil
    }
}
