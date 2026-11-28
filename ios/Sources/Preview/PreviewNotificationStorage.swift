#if DEBUG
import Foundation

@MainActor
final class PreviewPushReceiptStore: PushReceiptStore {
    private var receipt: PushReceipt?
    func load() throws -> PushReceipt? { receipt }
    func save(_ receipt: PushReceipt?) throws { self.receipt = receipt }
}

@MainActor
final class InMemoryNotificationPreviewKeys: NotificationPreviewKeyStorage {
    private var key: NotificationPreviewKey?
    func load() -> NotificationPreviewKey? { key }
    func save(_ key: NotificationPreviewKey?) { self.key = key }
}
#endif
