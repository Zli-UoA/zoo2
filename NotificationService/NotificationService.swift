import Foundation
import UserNotifications

final class NotificationService: UNNotificationServiceExtension {
    private struct Delivery {
        let id = UUID()
        let fallback: UNNotificationContent
        let handler: (UNNotificationContent) -> Void
    }

    private let lock = NSLock()
    private var pending: Delivery?

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        let fallback = NotificationPreview.content(for: request.content, key: nil)
        let delivery = Delivery(fallback: fallback, handler: contentHandler)
        let previous = lock.withLock {
            let previous = pending
            pending = delivery
            return previous
        }
        if let previous { previous.handler(previous.fallback) }

        let key = try? NotificationPreviewKeyStore().load()
        let content = NotificationPreview.content(for: fallback, key: key)
        let handler = lock.withLock {
            guard pending?.id == delivery.id else { return nil as ((UNNotificationContent) -> Void)? }
            pending = nil
            return delivery.handler
        }
        handler?(content)
    }

    override func serviceExtensionTimeWillExpire() {
        // Keychain の読み取り中に期限を迎えても、汎用通知を一度だけ渡す。
        let delivery = lock.withLock {
            let delivery = pending
            pending = nil
            return delivery
        }
        if let delivery { delivery.handler(delivery.fallback) }
    }
}
