import Foundation

@MainActor
protocol NotificationPreviewKeyStorage {
    func load() throws -> NotificationPreviewKey?
    func save(_ key: NotificationPreviewKey?) throws
}

extension NotificationPreviewKeyStore: NotificationPreviewKeyStorage {}
