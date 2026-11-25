import CryptoKit
import Foundation
import Testing
import ZooCrypto
@testable import Zoo

private let channel = "00000000-0000-4000-8000-0000000000c1"
private let messageID = "00000000-0000-4000-8000-0000000000d1"

/// 鍵バックアップの送信と取り込み（docs/e2ee.md の「鍵バックアップ」）。状態を持つ模擬サーバーと、実際の zoo-crypto。
@MainActor
struct KeyBackupTests {
    private struct Sent {
        let server: FakeSetupServer
        let sender: Device
        let capture: CapturingRequests
        let row: EncryptedRow
        let recoveryKey: String
    }

    /// 端末 A が初回設定をして、1 通を暗号化して送る（A の受信用セッションが 1 つ、送信待ちになる）。
    private func sendOne() async throws -> Sent {
        let server = FakeSetupServer(userID: user)
        let sender = try Device(server)
        let start = try sender.setup.begin()
        try await sender.setup.completeSetup(start.session, reauth: "t")
        let capture = CapturingRequests(user: user, store: { sender.store }, setupServer: server)
        try await MessageEncryption(store: { sender.store }, api: capture).send(channel: channel, id: messageID, parent: nil, body: "バックアップの本文")
        capture.record(revision: 0)
        let row = EncryptedRow(channelID: channel, id: messageID, parentID: nil, userID: user, kind: "message",
                               epoch: capture.epoch(0), revision: 0, ciphertext: capture.ciphertext(0))
        return Sent(server: server, sender: sender, capture: capture, row: row, recoveryKey: start.recoveryKey)
    }

    @Test func 送信待ちのセッションを今の版に送り送信済みにする() async throws {
        let sent = try await sendOne()
        defer { sent.sender.cleanUp() }
        // 初回設定を終えたとき、送信待ちはなかった。暗号化して送ったので、受信用セッションが 1 つ増えている。
        _ = try await sent.sender.setup.uploadKeyBackup()
        #expect(sent.server.backupRows.count == 1)
        #expect(sent.sender.store.pendingBackupCount() == 0)
        // 送るものがなければ、何も送らない。
        let uploadsBefore = sent.server.backupUploads.count
        #expect(try await sent.sender.setup.uploadKeyBackup() == .done(0))
        #expect(sent.server.backupUploads.count == uploadsBefore)
        // 今の版に `upload_key` がないサーバー（証明を足す前）には、証明を付けない。
        #expect(sent.server.backupProofs.allSatisfy { $0 == nil })
    }

    @Test func 今の版にupload_keyがあれば証明を付けて送る() async throws {
        let sent = try await sendOne()
        defer { sent.sender.cleanUp() }
        let publicKey = Curve25519.KeyAgreement.PrivateKey().publicKey.rawRepresentation
        sent.server.uploadKey = .string(Data(publicKey).base64EncodedString().replacingOccurrences(of: "=", with: ""))
        // 版の署名の検証に使うので、版を作り直さない（今の版のまま `upload_key` だけを足す）。
        #expect(try await sent.sender.setup.uploadKeyBackup() == .done(1))
        let proof = try #require(sent.server.backupProofs.last ?? nil)
        #expect(!proof.isEmpty)
    }

    @Test func upload_keyが文字列でなければ送らず送信待ちのまま残す() async throws {
        let sent = try await sendOne()
        defer { sent.sender.cleanUp() }
        sent.server.uploadKey = .null
        let result = try await sent.sender.setup.uploadKeyBackup()
        guard case .rejected = result else { Issue.record("rejected のはず: \(result)"); return }
        #expect(sent.server.backupRows.isEmpty)
        #expect(sent.sender.store.pendingBackupCount() == 1)
    }

    @Test func サーバーが版の違いで拒んだら送信待ちのまま残す() async throws {
        let sent = try await sendOne()
        defer { sent.sender.cleanUp() }
        sent.server.backupConflict = "E2EE_WRONG_BACKUP_VERSION"
        #expect(try await sent.sender.setup.uploadKeyBackup() == .conflict("E2EE_WRONG_BACKUP_VERSION"))
        #expect(sent.sender.store.pendingBackupCount() == 1)
        // 拒まれなくなれば、次に送れる。
        sent.server.backupConflict = nil
        #expect(try await sent.sender.setup.uploadKeyBackup() == .done(1))
        #expect(sent.sender.store.pendingBackupCount() == 0)
    }

    @Test func 新しい端末は解錠すると鍵バックアップから取り込み本文を読める() async throws {
        let sent = try await sendOne()
        let receiver = try Device(sent.server)
        defer { sent.sender.cleanUp(); receiver.cleanUp() }
        _ = try await sent.sender.setup.uploadKeyBackup()
        // 解錠する前は、鍵がないので読めない。
        let reader = MessageEncryption(store: { receiver.store }, api: sent.capture)
        #expect(reader.decrypt(sent.row).body == nil)
        // 解錠すると（to-device の鍵がなくても）、鍵バックアップから過去のセッションを取り込む。
        _ = try await receiver.setup.unlock(.recovery(sent.recoveryKey))
        #expect(receiver.store.inboundSessionCount == 1)
        let decrypted = reader.decrypt(sent.row)
        #expect(decrypted.body == "バックアップの本文")
        // バックアップ由来のセッションは、送信者を確かめられない。
        #expect(decrypted.notice == E2eeText.senderUnauthenticated)
        // 取り込んだセッションは、この版から取ったので、送信待ちに入れない。
        #expect(receiver.store.pendingBackupCount() == 0)
        // 同じ版から取り込み終えていれば、もう取り込まない。
        #expect(try await receiver.setup.importKeyBackup() == 0)
    }

    @Test func 解錠していない端末は送信も取り込みもしない() async throws {
        let server = FakeSetupServer(userID: user)
        let device = try Device(server)
        defer { device.cleanUp() }
        #expect(try await device.setup.uploadKeyBackup() == .done(0))
        #expect(try await device.setup.importKeyBackup() == 0)
        #expect(server.backupRows.isEmpty)
    }
}
