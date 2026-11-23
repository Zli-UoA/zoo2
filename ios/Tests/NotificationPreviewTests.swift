import Foundation
import Security
import Testing
import UserNotifications
@testable import Zoo

struct NotificationPreviewTests {
    private let userID = "00000000-0000-4000-8000-000000000001"
    private let noticeID = "00000000-0000-4000-8000-000000000002"
    private let channelID = "00000000-0000-4000-8000-000000000003"
    private let messageID = "00000000-0000-4000-8000-000000000004"
    private let secret = String(repeating: "ab", count: 32)
    // Node.js の crypto で生成した共通ベクトル。サーバーと端末で文字コードと暗号文の並びを確認する。
    private let reaction = "AAECAwQFBgcICQoLC1FCs/PJe1i1A/cNMCU14PcyVg4nlyrikzDzEQM0+B/vf/jqRnOjmpc22pNE3kgqULM2qOy1rzRitOxPOzcTSvakUaXieEPiKt88VAuGz+TzYtMHdNxD5r9zWKOTGYfmRh/nfKMCFuCUsDopi3K9DP6H/eY="
    private let mention = "JCUmJygpKissLS4vD4Fk3qntR1HNVIKVFyijlgbQVN6MJ6VNz9vO23bB/DVkobz72V/4UlZvNSfYy8rbCaoanN5sW2r+ZDU6sFWKqeMEdfoLcMgypADl2r8S2IsptVry6kXfLVrOPOlNUKPJ2OulXg=="
    private let missedCall = "GBkaGxwdHh8gISIjQ4VzLt1LAWD90f3sqQ7OPBMgrlMA3YRRbgC4Ck4LUzJ/ns+wkoPq+F/Mb4ypBZ4Nw51NArzd3GiQ/XHKVZeg0YdPndHRh5na/x4lvGmYXRF20PHKgIuVmpeckwU="
    private let selfTest = "DA0ODxAREhMUFRYXZOmYrybUCxm3Qxu8W1p2OhG5xM2X+ETPawz+9AeddmbddV2KhIxQ1sw6XRvurPNrG8U8qJs5Q8kiWsBRwqJqqFBO9mlgXO9Z6+B2bbKLlJEWEAQQVN2OJS+pnfhYSthccv7/xn5lmqMRRDYAPV1Ywm57zBbNEO5cL3nkdTSy"

    private var key: NotificationPreviewKey { NotificationPreviewKey(userID: userID, secret: secret) }

    private func content(_ preview: String) -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = "Zoo"
        content.body = "新しい通知があります"
        content.threadIdentifier = channelID
        content.userInfo = ["preview_v1": preview, "user_id": userID, "notification_id": noticeID,
                            "channel_id": channelID, "message_id": messageID]
        return content
    }

    @Test("無効化更新か鍵削除が成功すれば旧鍵を返さず、同時失敗は既知の制約として残る", arguments: [(false, true), (true, false), (true, true)])
    func invalidationAndDeletion(failures: (Bool, Bool)) throws {
        let live = NotificationPreviewKeyStore(account: "preview-test-" + UUID().uuidString)
        defer { try? live.save(nil) }
        try live.save(key)
        var failing = live
        if failures.0 { failing.updateItem = { _, _ in errSecInteractionNotAllowed } }
        if failures.1 { failing.deleteItem = { _ in errSecInteractionNotAllowed } }
        do {
            try failing.save(nil)
            #expect(!failures.1)
        } catch { #expect(failures.1) }
        #expect(try live.load() == (failures.0 && failures.1 ? key : nil))
    }

    @Test("端末登録の復号鍵とリアクション表示が共通ベクトルに一致する")
    func reactionPreview() {
        #expect(key.key.base64EncodedString() == "8GEHd+IGHGDsPw2yoAhdblQ8XGyN7Co2SD7CaahSJVw=")
        let rendered = NotificationPreview.content(for: content(reaction), key: key)
        #expect(rendered.title == "テスト · #動物園")
        #expect(rendered.body == "リアクション: 明日🐼を見に行く")
        #expect(rendered.threadIdentifier == channelID)
        #expect(rendered.userInfo["message_id"] as? String == messageID)
        #expect(rendered.userInfo[NotificationPreview.renderedKey] as? Bool == true)
    }

    @Test("暗号化チャンネルのメンションと不在着信はWebと同じ本文を表示する")
    func genericAndMissedCall() {
        #expect(NotificationPreview.content(for: content(mention), key: key).body == "新しいメッセージ")
        let original = content(missedCall)
        original.userInfo["message_id"] = NSNull()
        let rendered = NotificationPreview.content(for: original, key: key)
        #expect(rendered.body == "不在着信")
        #expect(rendered.userInfo[NotificationPreview.renderedKey] as? Bool == true)
    }

    @Test("テスト通知は通常通知と異なる識別子を照合する")
    func testNotice() {
        let original = content(selfTest)
        original.userInfo = ["user_id": userID, "type": "push_test", "test_id": noticeID, "preview_v1": selfTest]
        let rendered = NotificationPreview.content(for: original, key: key)
        #expect(rendered.body == "このiPhoneでZooの通知を受信できました。")
        #expect(rendered.userInfo[NotificationPreview.renderedKey] as? Bool == true)
        original.userInfo["channel_id"] = channelID
        #expect(NotificationPreview.content(for: original, key: key).body == original.body)
    }

    @Test("以前のアカウントや世代の鍵、未登録の端末では送信者と本文を表示しない")
    func differentRegistration() {
        let original = content(reaction)
        original.userInfo[NotificationPreview.renderedKey] = true
        let wrongUser = NotificationPreviewKey(userID: messageID, secret: secret)
        let oldKey = NotificationPreviewKey(userID: userID, secret: String(repeating: "12", count: 32))
        for unavailable in [nil, wrongUser, oldKey] as [NotificationPreviewKey?] {
            let result = NotificationPreview.content(for: original, key: unavailable)
            #expect(result.title == original.title)
            #expect(result.body == original.body)
            #expect(result.userInfo[NotificationPreview.renderedKey] == nil)
        }
    }

    @Test("転記、暗号文の改ざん、過大なデータは汎用文面に戻す")
    func untrustedPayload() throws {
        var modified = try #require(Data(base64Encoded: reaction))
        modified[modified.startIndex] ^= 1
        for (field, value) in [
            ("user_id", channelID), ("notification_id", messageID), ("channel_id", userID),
            ("message_id", noticeID), ("message_id", "不正なID"), ("type", "push_test"),
            ("preview_v1", modified.base64EncodedString()), ("preview_v1", "%"),
            ("preview_v1", String(repeating: "A", count: 4097))
        ] {
            let original = content(reaction)
            original.userInfo[field] = value
            let title = original.title, body = original.body
            let rendered = NotificationPreview.content(for: original, key: key)
            #expect(rendered.title == title)
            #expect(rendered.body == body)
            #expect(rendered.userInfo[NotificationPreview.renderedKey] == nil)
        }
    }
}
