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

    @Test("解錠後のバックアップ中に端末が変わったら解錠成功にしない", arguments: [false, true])
    func unlockRequiresStableDevice(close: Bool) async throws {
        let sent = try await sendOne(), receiver = try Device(sent.server)
        defer { sent.sender.cleanUp(); receiver.cleanUp() }
        var changed = false
        sent.server.beforeCurrentBackup = {
            if receiver.store.hasOwnKeys, !changed {
                changed = true
                if close { receiver.store.close() } else { try receiver.store.replaceDevice() }
            }
        }
        var waitedForInvalidStore = false
        let setup = E2eeSetup(api: sent.server, userID: user, store: {
            if !receiver.store.isUsable {
                waitedForInvalidStore = true
                throw CancellationError()
            }
            return receiver.store
        })
        await #expect(throws: E2eeSetupError.secretsChanged) {
            try await setup.unlock(.recovery(sent.recoveryKey))
        }
        #expect(changed && !waitedForInvalidStore)
    }

    @Test("バックアップ版の競合後は現在版を読み直して再試行する")
    func scheduledBackupRetriesVersionConflict() async throws {
        let sent = try await sendOne()
        defer { sent.sender.cleanUp() }
        let before = sent.server.currentBackupReads
        var attempts = 0
        sent.server.beforeBackupUpload = {
            attempts += 1
            if attempts == 1 { throw E2eeAPIError(status: 409, code: "E2EE_WRONG_BACKUP_VERSION") }
        }
        sent.sender.setup.scheduleKeyBackup(retryDelay: .zero)
        defer { sent.sender.setup.cancelScheduledBackup() }
        for _ in 0..<100 where sent.server.backupRows.isEmpty { try await Task.sleep(for: .milliseconds(10)) }
        #expect(attempts == 2 && !sent.server.backupRows.isEmpty)
        #expect(sent.server.currentBackupReads >= before + 2)
    }

    @Test("送信後のバックアップ失敗は鍵だけを再試行する")
    func retriesBackupWithoutReposting() async throws {
        let sent = try await sendOne()
        defer { sent.sender.cleanUp() }
        sent.server.failNext.insert("backup")
        let encryption = MessageEncryption(store: { sent.sender.store }, api: sent.capture, uploadBackup: {
            sent.sender.setup.scheduleKeyBackup(retryDelay: .milliseconds(1))
        })
        try await encryption.send(channel: channel, id: UUID().uuidString.lowercased(), parent: nil, body: "二重送信しない")
        let requestsAfterSend = sent.capture.requests.count
        await sent.sender.setup.scheduledBackup?.value
        #expect(!sent.server.failNext.contains("backup"))
        #expect(sent.server.backupRows.count == 2)
        #expect(sent.sender.store.pendingBackupCount() == 0)
        #expect(sent.capture.requests.count == requestsAfterSend)
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

    @Test(arguments: [#"{"sessions":null,"next":null}"#, #"{"sessions":[],"next":5}"#, #"{"sessions":[],"next":"A"}"#])
    func 不正なページや繰り返すカーソルでは取り込み完了を記録しない(page: String) async throws {
        let sent = try await sendOne()
        let receiver = try Device(sent.server)
        defer { sent.sender.cleanUp(); receiver.cleanUp() }
        _ = try await sent.sender.setup.uploadKeyBackup()
        sent.server.backupPage = { _ in page }
        // 不正なページで取り込みが失敗する（完了を記録しない）。
        _ = try? await receiver.setup.unlock(.recovery(sent.recoveryKey))
        #expect(receiver.store.inboundSessionCount == 0)
        // 完了を記録していないので、正常なページなら取り込める。
        sent.server.backupPage = nil
        #expect(try await receiver.setup.importKeyBackup() == 1)
    }

    @Test("型が壊れたバックアップ行だけを除外し、同じページでは差分を再保存しない")
    func ignoresMalformedRowsAndUnchangedPages() async throws {
        let sent = try await sendOne(), receiver = try Device(sent.server)
        defer { sent.sender.cleanUp(); receiver.cleanUp() }
        _ = try await receiver.setup.unlock(.recovery(sent.recoveryKey))
        _ = try await sent.sender.setup.uploadKeyBackup()
        let row = try #require(sent.server.backupRows.values.first)
        let text = String(decoding: try JSONEncoder().encode(row), as: UTF8.self)
        func pages() throws -> [String] {
            try FileManager.default.contentsOfDirectory(atPath: receiver.files.directory.path)
                .filter { $0.hasPrefix("backup-page-") }.sorted()
        }
        #expect(try receiver.store.importBackupRows(["{}", text, "壊れたJSON"]) == 1)
        #expect(receiver.store.isUsable && receiver.store.inboundSessionCount == 1)
        let written = try pages()
        try #require(written.count == 1)
        #expect(try receiver.store.importBackupRows([text, text]) == 2)
        #expect(try pages() == written)
        try receiver.restart()
        #expect(try receiver.store.importBackupRows([text]) == 1)
        #expect(try pages() == written)
    }

    @Test("バックアップは変更分だけを書き、途中から再開できる", arguments: [10, 100])
    func incrementalBackupPages(pageCount: Int) async throws {
        let sent = try await sendOne()
        let receiver = try Device(sent.server)
        defer { sent.sender.cleanUp(); receiver.cleanUp() }
        // 空のバックアップで解錠し、ページの取り込みを個別に進める。
        _ = try await receiver.setup.unlock(.recovery(sent.recoveryKey))
        for n in 1..<pageCount {
            let session = try OutboundSession(channelId: channel, epoch: 1, nowMs: UInt64(n))
            _ = try sent.sender.store.encrypt(session, senderKey: sent.sender.store.curve25519Key,
                message: OutgoingMessage(messageId: UUID().uuidString.lowercased(), parentId: nil, revision: 0,
                    content: ZooCrypto.MessageContent(body: "復元対象", format: "mfm", kind: "message"),
                    mentions: Mentions(userIds: [], channel: false, here: false), attachments: []))
        }
        _ = try await sent.sender.setup.uploadKeyBackup()
        let rows = sent.server.backupRows.values.map { String(decoding: try! JSONEncoder().encode($0), as: UTF8.self) }
        try #require(rows.count == pageCount)
        let stateURL = receiver.files.directory.appendingPathComponent(CryptoStore.stateFile)
        let initial = try Data(contentsOf: stateURL)
        for (index, row) in rows.enumerated() {
            #expect(try receiver.store.importBackupRows([row]) == 1)
            if index == pageCount / 2 - 1 {
                try receiver.restart()
                #expect(receiver.store.inboundSessionCount == pageCount / 2)
            }
        }
        #expect(try Data(contentsOf: stateURL) == initial)
        let pages = try FileManager.default.contentsOfDirectory(at: receiver.files.directory, includingPropertiesForKeys: nil)
            .filter { $0.lastPathComponent.hasPrefix("backup-page-") && $0.pathExtension == "json" }
        #expect(pages.count == pageCount)
        let sizes = try pages.map { try Data(contentsOf: $0).count }
        #expect(try #require(sizes.max()) < 2 * #require(sizes.min()))
        let total = sizes.reduce(0, +)
        try receiver.store.backupImported("検証用の完了版")
        let snapshotSize = try Data(contentsOf: stateURL).count
        #expect(total < snapshotSize * 2)
        print("バックアップ\(pageCount)ページ: 差分書込=\(total) bytes、最終状態=\(snapshotSize) bytes")
        try receiver.restart()
        #expect(receiver.store.inboundSessionCount == pageCount)
        #expect(MessageEncryption(store: { receiver.store }, api: sent.capture).decrypt(sent.row).body == "バックアップの本文")
    }

    @Test("ページ保存の前後で失敗したストアは再読込してから再開する", arguments: [false, true])
    func backupPageFailureRequiresReopen(afterRename: Bool) async throws {
        let sent = try await sendOne(), receiver = try Device(sent.server)
        defer { sent.sender.cleanUp(); receiver.cleanUp() }
        _ = try await receiver.setup.unlock(.recovery(sent.recoveryKey))
        _ = try await sent.sender.setup.uploadKeyBackup()
        let row = try #require(sent.server.backupRows.values.first)
        let text = String(decoding: try JSONEncoder().encode(row), as: UTF8.self)
        let path = receiver.files.directory.standardizedFileURL.path
        CryptoStore.writeFailureHooks.withLock { hooks in hooks[path] = { url, renamed in
            if url.lastPathComponent.hasPrefix("backup-page-"), renamed == afterRename { throw CocoaError(.fileWriteUnknown) }
        } }
        defer { CryptoStore.writeFailureHooks.withLock { $0.removeValue(forKey: path) } }
        #expect(throws: (any Error).self) { try receiver.store.importBackupRows([text]) }
        #expect(!receiver.store.isUsable)
        #expect(throws: E2eeStoreError.closed) { try receiver.store.importBackupRows([text]) }
        CryptoStore.writeFailureHooks.withLock { $0.removeValue(forKey: path) }
        try receiver.restart()
        #expect(receiver.store.inboundSessionCount == (afterRename ? 1 : 0))
        _ = try receiver.store.importBackupRows([text])
        #expect(receiver.store.inboundSessionCount == 1)
        try receiver.store.backupImported("再開済み")
        try receiver.restart()
        #expect(MessageEncryption(store: { receiver.store }, api: sent.capture).decrypt(sent.row).body == "バックアップの本文")
    }

    @Test("集約済みの古いページや別端末のページを再適用しない")
    func ignoresCompactedAndOtherDevicePages() async throws {
        let sent = try await sendOne(), receiver = try Device(sent.server)
        defer { sent.sender.cleanUp(); receiver.cleanUp() }
        _ = try await receiver.setup.unlock(.recovery(sent.recoveryKey))
        _ = try await sent.sender.setup.uploadKeyBackup()
        let row = try #require(sent.server.backupRows.values.first)
        _ = try receiver.store.importBackupRows([String(decoding: try JSONEncoder().encode(row), as: UTF8.self)])
        let page = try #require(FileManager.default.contentsOfDirectory(at: receiver.files.directory, includingPropertiesForKeys: nil)
            .first { $0.lastPathComponent.hasPrefix("backup-page-") && $0.pathExtension == "json" })
        try receiver.store.backupImported("集約済み")
        // 削除失敗で残ったファイルは、壊れていても集約済みなら復号状態に戻さない。
        try Data("壊れた旧ページ".utf8).write(to: page)
        try receiver.restart()
        #expect(receiver.store.inboundSessionCount == 1)
        try receiver.store.replaceDevice()
        try Data("別端末の旧ページ".utf8).write(to: page)
        try receiver.restart()
        #expect(receiver.store.inboundSessionCount == 1)
    }

    @Test("集約前のページの欠落や内容の不一致を拒否する", arguments: ["番号", "端末", "欠落"])
    func rejectsBrokenBackupJournal(reason: String) async throws {
        let sent = try await sendOne(), receiver = try Device(sent.server)
        defer { sent.sender.cleanUp(); receiver.cleanUp() }
        _ = try await receiver.setup.unlock(.recovery(sent.recoveryKey))
        _ = try await sent.sender.setup.uploadKeyBackup()
        let row = try #require(sent.server.backupRows.values.first)
        let text = String(decoding: try JSONEncoder().encode(row), as: UTF8.self)
        #expect(try receiver.store.importBackupRows([text]) == 1)
        let page = try #require(FileManager.default.contentsOfDirectory(at: receiver.files.directory, includingPropertiesForKeys: nil)
            .first { $0.lastPathComponent.hasPrefix("backup-page-") && $0.pathExtension == "json" })
        receiver.store.close()
        var value = try #require(JSONSerialization.jsonObject(with: Data(contentsOf: page)) as? [String: Any])
        if reason == "端末" { value["device_id"] = "別の端末" }
        else { value["sequence"] = 2 }
        try JSONSerialization.data(withJSONObject: value).write(to: page)
        if reason == "欠落" {
            let destination = page.deletingLastPathComponent().appendingPathComponent(
                page.lastPathComponent.replacingOccurrences(of: "00000000000000000001.json", with: "00000000000000000002.json"))
            try FileManager.default.moveItem(at: page, to: destination)
        }
        #expect(throws: (any Error).self) { try receiver.restart() }
    }

    @Test func SSSSの競合のあとの読み直しに失敗したら解錠済みを返さず投げる() async throws {
        let sent = try await sendOne()
        defer { sent.sender.cleanUp() }
        sent.server.backupConflict = "E2EE_SSSS_CONFLICT"
        // 起動時の確認の取得は成功し、競合のあとの読み直しだけ失敗する（1 回目の取得のあとに失敗を仕込む）。
        sent.server.failSecretStorageAfterFirst = true
        await #expect(throws: E2eeSetupError.secretsChanged) { _ = try await sent.sender.setup.runStartupCheck() }
    }

    @Test func 起動時の取り込み中に版だけが変わった場合も状態を読み直す() async throws {
        let server = FakeSetupServer(userID: user)
        let device = try Device(server)
        defer { device.cleanUp() }
        let start = try device.setup.begin()
        try await device.setup.completeSetup(start.session, reauth: "t")
        var readPage = false
        server.backupPage = { _ in
            readPage = true
            // 自分の鍵は同じまま、別の処理がローカルの版を更新した直後を再現する。
            do { try device.store.saveOwnKeys(start.session, secretsRevision: server.revision + 1) }
            catch { Issue.record(error) }
            return #"{"sessions":[],"next":null}"#
        }
        let status = try await device.setup.runStartupCheck()
        #expect(readPage)
        #expect(status != .unlocked)
        #expect(device.store.hasOwnKeys && device.store.secretsRevision == nil)
    }
}
