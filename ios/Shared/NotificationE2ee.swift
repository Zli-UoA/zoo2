import Foundation
import UserNotifications
import ZooCrypto

/// 暗号化チャンネルの通知を、通知拡張で復号する（docs/e2ee.md の「iOS の通知の復号」。#685、#510）。
///
/// サーバーは、暗号化チャンネルの通知に暗号文の行（`e2ee_v1`）を付ける。拡張は、App Group にある暗号ストアの
/// `state.json` を読み取り専用で開き、共有の Keychain の保存鍵で受信用セッションを開いて、受信時の照合を通して復号する。
/// 状態は書き戻さない（本体が同時に動いていても壊さない。`state.json` は一時ファイルから `rename` で置き換わるので、
/// 読むときは、前か次のどちらかの完全な状態）。復号できなければ `nil`（汎用の通知のまま出る）。
enum NotificationE2ee {
    /// 通知拡張と暗号ストアを共有する App Group。
    static let appGroup = "group.works.zli.zoo"

    /// 通知拡張が暗号ストアを探す置き場の候補。
    static func roots() -> [URL] {
        guard let group = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup) else { return [] }
        return [group.appendingPathComponent("e2ee", isDirectory: true)]
    }

    /// 通知の本文の上限（コードポイント）。平文チャンネルと同じ。
    static let bodyLimit = 240

    private struct State: Decodable {
        struct Inbound: Decodable { let pickle: String }
        let user_id: String
        let device_id: String
        let account: String
        let inbound_sessions: [String: Inbound]?
    }

    /// 通知の `userInfo` から復号した本文。`roots` は暗号ストアの置き場の候補（`<ID>/state.json` が並ぶディレクトリー）、
    /// `loadKey` は暗号ストアの ID（ディレクトリー名）から保存鍵を読む。
    static func body(userInfo info: [AnyHashable: Any], roots: [URL], loadKey: (String) -> Data?) -> String? {
        guard let user = info["user_id"] as? String, UUID(uuidString: user) != nil,
              let e2ee = info["e2ee_v1"] as? [String: Any],
              let recipient = e2ee["recipient_id"] as? String, recipient.lowercased() == user.lowercased(),
              let row = e2ee["message"] as? [String: Any],
              let channelID = row["channel_id"] as? String, let messageID = row["message_id"] as? String,
              let sender = row["user_id"] as? String, let kind = row["message_kind"] as? String,
              let epoch = (row["epoch"] as? NSNumber)?.uint64Value, let revision = (row["revision"] as? NSNumber)?.uint64Value,
              let ciphertext = row["ciphertext"], JSONSerialization.isValidJSONObject(ciphertext),
              let cipherData = try? JSONSerialization.data(withJSONObject: ciphertext, options: [.sortedKeys]),
              let cipherText = String(data: cipherData, encoding: .utf8),
              let envelope = ciphertext as? [String: Any],
              let senderKey = envelope["sender_key"] as? String, let sessionID = envelope["session_id"] as? String else { return nil }
        let parentID = row["parent_id"] as? String

        for root in roots {
            let directories = (try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
            for directory in directories {
                guard let data = try? Data(contentsOf: directory.appendingPathComponent("state.json")),
                      let state = try? JSONDecoder().decode(State.self, from: data),
                      state.user_id.lowercased() == recipient.lowercased(),
                      let storageKey = loadKey(directory.lastPathComponent), storageKey.count == 32 else { continue }
                let key = String(decoding: (try? JSONEncoder().encode([channelID, senderKey, sessionID])) ?? Data(), as: UTF8.self)
                guard let stored = state.inbound_sessions?[key],
                      let account = try? DeviceAccount.fromPickle(pickle: state.account, storageKey: storageKey,
                                                                  userId: state.user_id, deviceId: state.device_id) else { continue }
                let sessions = InboundSessionStore()
                guard (try? sessions.restoreSession(account: account, pickle: stored.pickle, storageKey: storageKey)) != nil,
                      let decrypted = try? sessions.decrypt(message: EncryptedMessage(
                        channelId: channelID, messageId: messageID, parentId: parentID, userId: sender, messageKind: kind,
                        epoch: epoch, revision: revision, ciphertext: cipherText)),
                      decrypted.revision == .latest else { continue }
                return String(String.UnicodeScalarView(decrypted.payload.content.body.unicodeScalars.prefix(bodyLimit)))
            }
        }
        return nil
    }

    /// 通知の内容に、復号した本文を入れる。リアクションの通知は、今までどおり「リアクション: 」を前に付ける。
    static func content(for original: UNNotificationContent, roots: [URL], loadKey: (String) -> Data?) -> UNNotificationContent {
        guard let body = body(userInfo: original.userInfo, roots: roots, loadKey: loadKey),
              let content = original.mutableCopy() as? UNMutableNotificationContent else { return original }
        // プレビューが「リアクション: 」を付けていたら（リアクションの通知）、復号した本文にも付ける。
        let isReaction = original.body.hasPrefix("リアクション: ")
        content.body = isReaction ? "リアクション: " + body : body
        return content
    }
}
