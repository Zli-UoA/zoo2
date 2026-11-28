import CryptoKit
import Foundation
import Security
import UserNotifications

struct NotificationPreviewKey: Codable, Equatable, Sendable {
    let userID: String
    let key: Data

    init(userID: String, secret: String) {
        self.userID = userID
        // サーバーが保持する解除情報のハッシュから、復号専用の鍵だけを拡張に渡す。
        let verifier = SHA256.hash(data: Data(secret.utf8)).map { String(format: "%02x", $0) }.joined()
        key = Data(HMAC<SHA256>.authenticationCode(
            for: Data("works.zli.zoo.apns.preview.v1".utf8),
            using: SymmetricKey(data: Data(verifier.utf8))
        ))
    }
}

struct NotificationPreviewKeyStore {
    var account = "current"
    var updateItem: @Sendable (CFDictionary, CFDictionary) -> OSStatus = { SecItemUpdate($0, $1) }
    var deleteItem: @Sendable (CFDictionary) -> OSStatus = { SecItemDelete($0) }
    private func query() throws -> [String: Any] {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "ZooPreviewKeychainGroup") as? String,
              !group.isEmpty, !group.contains("$(") else { throw error(errSecMissingEntitlement) }
        return [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "works.zli.zoo.notification-preview",
            kSecAttrAccount as String: account,
            kSecAttrAccessGroup as String: group
        ]
    }

    private func error(_ status: OSStatus) -> NSError {
        NSError(domain: NSOSStatusErrorDomain, code: Int(status))
    }

    func load() throws -> NotificationPreviewKey? {
        var request = try query()
        request[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw error(status) }
        guard let data = result as? Data else { throw error(errSecDecode) }
        return try JSONDecoder().decode(NotificationPreviewKey?.self, from: data)
    }

    func save(_ key: NotificationPreviewKey?) throws {
        let query = try query()
        guard let key else {
            // 削除だけが失敗しても復号を止めるため、先に同じ共有項目を鍵なしへ更新する。
            _ = updateItem(query as CFDictionary, [kSecValueData as String: Data("null".utf8)] as CFDictionary)
            let status = deleteItem(query as CFDictionary)
            guard status == errSecSuccess || status == errSecItemNotFound else { throw error(status) }
            return
        }
        let attributes: [String: Any] = [
            kSecValueData as String: try JSONEncoder().encode(key),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        ]
        var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            status = SecItemAdd(query.merging(attributes) { _, new in new } as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw error(status) }
    }
}

enum NotificationPreview {
    static let renderedKey = "zoo_preview_rendered"

    private struct Text: Decodable {
        let title: String
        let body: String
        let notificationType: String
    }

    static func content(for original: UNNotificationContent, key: NotificationPreviewKey?) -> UNNotificationContent {
        guard let content = original.mutableCopy() as? UNMutableNotificationContent else { return original }
        content.userInfo.removeValue(forKey: renderedKey)
        let info = content.userInfo
        guard let key, key.key.count == 32,
              let user = info["user_id"] as? String, let recipientID = UUID(uuidString: user),
              recipientID == UUID(uuidString: key.userID),
              let encoded = info["preview_v1"] as? String, encoded.utf8.count <= 4096,
              let combined = Data(base64Encoded: encoded),
              let box = try? AES.GCM.SealedBox(combined: combined) else { return content }

        let notice: String
        let channel: String
        let message: String
        let isTest = info["type"] as? String == "push_test"
        if isTest {
            guard let testID = info["test_id"] as? String, UUID(uuidString: testID) != nil,
                  info["notification_id"] == nil, info["channel_id"] == nil, info["message_id"] == nil else { return content }
            notice = testID
            channel = ""
            message = ""
        } else {
            guard info["type"] == nil, info["test_id"] == nil,
                  let noticeID = info["notification_id"] as? String, UUID(uuidString: noticeID) != nil,
                  let channelID = info["channel_id"] as? String, UUID(uuidString: channelID) != nil else { return content }
            notice = noticeID
            channel = channelID
            if let value = info["message_id"], !(value is NSNull) {
                guard let messageID = value as? String, UUID(uuidString: messageID) != nil else { return content }
                message = messageID
            } else {
                message = ""
            }
        }

        let aad = Data(["zoo-preview-v1", user, notice, channel, message].joined(separator: "|").utf8)
        guard let clear = try? AES.GCM.open(box, using: SymmetricKey(data: key.key), authenticating: aad),
              let text = try? JSONDecoder().decode(Text.self, from: clear),
              text.title.unicodeScalars.count <= 124, text.body.unicodeScalars.count <= 240 else { return content }
        switch text.notificationType {
        case "push_test" where isTest, "missed_call" where !isTest && message.isEmpty:
            break
        case "mention", "reply", "dm", "reaction", "channel_message":
            guard !isTest, !message.isEmpty else { return content }
        default:
            return content
        }
        content.title = text.title
        content.body = text.notificationType == "reaction" ? "リアクション: " + text.body : text.body
        content.userInfo[renderedKey] = true
        return content
    }
}
