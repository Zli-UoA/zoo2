import Foundation
import OSLog
import UserNotifications

struct IncomingCallNotificationAction: Sendable {
    let callID: UUID
    let userID: String
    let action: WatchAction

    init?(request: UNNotificationRequest, actionIdentifier: String) {
        guard request.content.categoryIdentifier == IncomingCallNotifications.categoryID,
              request.identifier.hasPrefix(IncomingCallNotifications.identifierPrefix),
              let id = request.content.userInfo["invite_id"] as? String, let callID = UUID(uuidString: id),
              let userID = request.content.userInfo["user_id"] as? String, UUID(uuidString: userID) != nil else { return nil }
        switch actionIdentifier {
        case IncomingCallNotifications.answerID: action = .answer
        case IncomingCallNotifications.declineID: action = .decline
        default: return nil
        }
        self.callID = callID
        self.userID = userID
    }
}

@MainActor
final class IncomingCallNotifications {
    nonisolated static let categoryID = "zoo.incoming-call"
    nonisolated static let identifierPrefix = "zoo.incoming-call."
    nonisolated static let answerID = "zoo.incoming-call.answer"
    nonisolated static let declineID = "zoo.incoming-call.decline"

    static var category: UNNotificationCategory {
        // 転送通知の操作をiPhoneで処理するため、foregroundは指定しない。
        UNNotificationCategory(identifier: categoryID, actions: [
            UNNotificationAction(identifier: answerID, title: "iPhoneで応答", options: [],
                                 icon: UNNotificationActionIcon(systemImageName: "phone.fill")),
            UNNotificationAction(identifier: declineID, title: "拒否", options: [.destructive],
                                 icon: UNNotificationActionIcon(systemImageName: "phone.down.fill"))
        ], intentIdentifiers: [], hiddenPreviewsBodyPlaceholder: "通話の着信です。", options: [])
    }

    private let add: @MainActor (UNNotificationRequest) async throws -> Void
    private let remove: @MainActor ([String]) -> Void
    private let existing: @MainActor () async -> [String]
    private var current: (callID: UUID, identifier: String)?
    private var delivery: Task<Void, Never>?

    init(add: @escaping @MainActor (UNNotificationRequest) async throws -> Void = {
        try await UNUserNotificationCenter.current().add($0)
    }, remove: @escaping @MainActor ([String]) -> Void = {
        let center = UNUserNotificationCenter.current()
        center.removePendingNotificationRequests(withIdentifiers: $0)
        center.removeDeliveredNotifications(withIdentifiers: $0)
    }, existing: @escaping @MainActor () async -> [String] = {
        let center = UNUserNotificationCenter.current()
        let pending = await center.pendingNotificationRequests()
        let delivered = await center.deliveredNotifications()
        return pending.map(\.identifier) + delivered.map { $0.request.identifier }
    }) {
        self.add = add
        self.remove = remove
        self.existing = existing
    }

    deinit { delivery?.cancel() }

    func removePreviousNotifications() async {
        let identifiers = await existing()
        // 取得を待つ間に新しい着信が届いても、その通知は残す。
        remove(identifiers.filter { $0.hasPrefix(Self.identifierPrefix) && $0 != current?.identifier })
    }

    func update(_ call: IncomingCall?, enabled: Bool) {
        let next = enabled && (call?.expiresAt ?? .distantPast) > .now ? call : nil
        guard next?.id != current?.callID else { return }
        delivery?.cancel()
        delivery = nil
        if let current { remove([current.identifier]) }
        current = nil
        guard let next else { return }
        // 再有効化後に古い登録処理が完了しても、新しい通知を削除しない。
        let identifier = Self.identifierPrefix + UUID().uuidString
        current = (next.id, identifier)
        let content = UNMutableNotificationContent()
        content.title = "着信"
        content.body = "\(next.callerName)からの着信です。"
        content.sound = .default
        content.categoryIdentifier = Self.categoryID
        content.userInfo = ["invite_id": next.id.uuidString, "user_id": next.userID]
        let request = UNNotificationRequest(identifier: identifier, content: content, trigger: nil)
        let add = add, remove = remove
        delivery = Task { [weak self] in
            guard !Task.isCancelled, next.expiresAt > .now else { return }
            do {
                try await add(request)
                guard !Task.isCancelled, self?.current?.identifier == identifier, next.expiresAt > .now else {
                    remove([identifier])
                    return
                }
            } catch {
                if self?.current?.identifier == identifier { self?.current = nil }
                Logger(subsystem: "works.zli.zoo", category: "IncomingCallNotifications")
                    .error("着信通知を登録できませんでした。")
            }
        }
    }
}
