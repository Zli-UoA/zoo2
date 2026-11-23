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

        let keys = NotificationPreviewKeyStore()
        let key = try? keys.load()
        let content = NotificationPreview.content(for: fallback, key: key)
        // 再照合中も期限切れの汎用通知を渡せるよう、配信権をまだ取り出さない。
        let valid = key != nil && (try? keys.load()) == key
        let handler = lock.withLock {
            guard pending?.id == delivery.id else { return nil as ((UNNotificationContent) -> Void)? }
            pending = nil
            return delivery.handler
        }
        // 復号中のログアウトや登録切替を、配信へ渡す直前にも照合する。
        handler?(valid ? content : fallback)
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
