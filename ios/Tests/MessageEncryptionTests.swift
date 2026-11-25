import Foundation
import Testing
import ZooCrypto
@testable import Zoo

private let channel = "00000000-0000-4000-8000-0000000000c1"
private let messageID = "00000000-0000-4000-8000-0000000000d1"

private func parse(_ text: String) -> JSONValue { try! JSONDecoder().decode(JSONValue.self, from: Data(text.utf8)) }
private func dump(_ value: JSONValue) -> String { String(decoding: try! JSONEncoder().encode(value), as: UTF8.self) }

/// 状態を持つ模擬サーバー（Android の `AndroidMessageEncryptionTest` と同じ）。暗号は実際の zoo-crypto。
@MainActor
private final class FakeMessageServer: E2eeRequesting {
    let setup: FakeSetupServer
    let sender: Device
    let receiver: Device
    var epoch: UInt64 = 1
    var posts = 0
    var requests: [String] = []
    var delivered: [CryptoStore.ToDeviceRow] = []
    var accepted: [String: JSONValue] = [:]
    var refuseDelivery = false
    /// `POST /api/messages` の最初の 1 回を、epoch が進んだ 409 にする。
    var staleOnce = true
    /// `to_device` の応答に `undelivered` がない（読めない応答）。
    var omitUndelivered = false
    /// `to_device` を処理する前に 1 回だけ呼ぶ（鍵の配布の応答を待つあいだに起きることを再現する）。
    var beforeToDevice: (() async throws -> Void)?
    /// 配布先を 2 ページに分ける（1 ページ目に `next_after` を付ける）。
    var pagedRecipients = false

    init(setup: FakeSetupServer, sender: Device, receiver: Device) { self.setup = setup; self.sender = sender; self.receiver = receiver }

    func snapshot() async throws -> [String: JSONValue] {
        guard case .object(var root) = parse(try await setup.queryOwnKeys(userID: user)) else { fatalError() }
        root["user_ids"] = .array([.string(user)])
        root["membership_epoch"] = .number(Double(epoch))
        var devices: [String: JSONValue] = [:]
        for device in [sender, receiver] { devices[device.store.deviceID] = parse(device.store.deviceKeys()) }
        root["device_keys"] = .object([user: .object(devices)])
        return root
    }

    var queries: [[String: String]] = []

    func e2eeJSON(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?) async throws -> JSONValue {
        requests.append(body.map { dump(.object($0)) } ?? "")
        queries.append(query)
        switch path {
        case "api/e2ee/channels/\(channel)/recipients":
            var page = try await snapshot()
            // 2 ページ目：同じ内容で、これ以上続かない。
            if pagedRecipients, query["after"] == nil { page["next_after"] = .string("page-2") }
            return .object(page)
        case "api/e2ee/keys/claim":
            let key = try #require(try receiver.store.prepareKeyUpload(oneTimeKeyCount: 0, hasUnusedFallbackKey: true)).oneTimeKeys.first!
            return .object(["one_time_keys": .object([user: .object([receiver.store.deviceID: .object([key.key: key.value])])])])
        case "api/e2ee/to_device":
            if let hook = beforeToDevice { beforeToDevice = nil; try await hook() }
            guard case .object(let messages)? = body?["messages"], case .object(let perUser)? = messages[user],
                  let content = perUser[receiver.store.deviceID] else { fatalError() }
            delivered.append(.init(id: String(delivered.count), senderUserID: user, type: "m.room.encrypted", content: dump(content)))
            if omitUndelivered { return .object([:]) }
            return .object(["undelivered": .array(refuseDelivery ? [.object(["user_id": .string(user), "device_id": .string(receiver.store.deviceID)])] : [])])
        case "api/messages", "api/messages/\(messageID)":
            posts += 1
            if staleOnce && posts == 1 {
                epoch = 2
                throw E2eeAPIError(status: 409, code: "E2EE_STALE_EPOCH", currentEpoch: 2)
            }
            accepted = body ?? [:]
            return .object([:])
        default:
            fatalError("Unexpected request: \(path)")
        }
    }
}

/// 配布先の `membership_epoch` が、整数で表せない値を返す。
@MainActor
private final class BrokenEpochServer: E2eeRequesting {
    let base: FakeMessageServer
    init(base: FakeMessageServer) { self.base = base }
    func e2eeJSON(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?) async throws -> JSONValue {
        guard path.hasPrefix("api/e2ee/channels/") else { return try await base.e2eeJSON(path, method: method, query: query, body: body) }
        guard case .object(var page) = try await base.e2eeJSON(path, method: method, query: query, body: body) else { return .object([:]) }
        page["membership_epoch"] = .number(1e20)
        return .object(page)
    }
}

@MainActor
struct MessageEncryptionTests {
    private func makePair() async throws -> (FakeMessageServer, Device, Device, MessageEncryption, String) {
        let setupServer = FakeSetupServer(userID: user)
        let sender = try Device(setupServer)
        let start = try sender.setup.begin()
        try await sender.setup.completeSetup(start.session, reauth: "t")
        let receiver = try Device(setupServer)
        _ = try await receiver.setup.unlock(.recovery(start.recoveryKey))
        let server = FakeMessageServer(setup: setupServer, sender: sender, receiver: receiver)
        let encryption = MessageEncryption(store: { sender.store }, api: server)
        return (server, sender, receiver, encryption, start.recoveryKey)
    }

    private func number(_ value: JSONValue?) -> UInt64 {
        if case .number(let number)? = value { return UInt64(number) }
        return 0
    }

    private func row(_ server: FakeMessageServer) -> EncryptedRow {
        var ciphertext = ""
        if case .string(let text)? = server.accepted["ciphertext"] { ciphertext = text }
        return EncryptedRow(channelID: channel, id: messageID, parentID: nil, userID: user, kind: "message",
                            epoch: number(server.accepted["epoch"]), revision: number(server.accepted["revision"]),
                            ciphertext: ciphertext)
    }

    @Test func 送信は平文を送らず鍵を配り受信端末が復号できる() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        let secret = "SECRET-IOS-MESSAGE"
        try await encryption.send(channel: channel, id: messageID, parent: nil, body: secret)
        // 最初の POST は epoch が進んだ 409 になり、新しい epoch で配り直して 2 回目で通る。
        #expect(server.posts == 2)
        #expect(server.requests.allSatisfy { !$0.contains(secret) })
        #expect(server.accepted["content"] == .string(""))
        #expect(server.accepted["epoch"] == .number(2))
        #expect(!server.delivered.isEmpty)
        // 受信端末が to-device で鍵を受け取り、送信端末を検証できる。
        let result = try receiver.store.processToDevice(server.delivered, keysQuery: dump(.object(try await server.snapshot())))
        #expect(result.roomKeys >= 1)
        #expect(result.verifiedRoomKeys >= 1)
        let reader = MessageEncryption(store: { receiver.store }, api: server)
        let decrypted = reader.decrypt(row(server))
        #expect(decrypted.body == secret)
        #expect(decrypted.notice == nil)
        // 別のチャンネルとして読むと、検証に失敗する。
        var other = row(server)
        other = EncryptedRow(channelID: "00000000-0000-4000-8000-0000000000c2", id: other.id, parentID: nil, userID: user,
                             kind: "message", epoch: other.epoch, revision: other.revision, ciphertext: other.ciphertext)
        #expect(reader.decrypt(other).body == nil)
        // アプリを終了して開き直しても、保存した受信用セッションで読める。
        try receiver.restart()
        let reopened = MessageEncryption(store: { receiver.store }, api: server)
        #expect(reopened.decrypt(row(server)).body == secret)
    }

    @Test func 暗号化した添付は暗号文の中にだけ入り受信端末が復号できる() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp(); EncryptedAttachmentFiles.shared.removeAll() }
        let plaintext = Data("SECRET-ATTACHMENT-BYTES".utf8)
        let encrypted = ZooCrypto.encryptAttachment(plaintext: plaintext)
        #expect(encrypted.ciphertext != plaintext)
        let attachmentID = "00000000-0000-4000-8000-0000000000e1"
        let info = AttachmentInfo(attachmentId: attachmentID, name: "秘密.txt", mimetype: "text/plain", size: UInt64(plaintext.count),
                                  width: nil, height: nil, sensitive: false, file: encrypted.file)
        try await encryption.send(channel: channel, id: messageID, parent: nil, body: "添付つき", attachments: [info])
        // 紐付けの ID だけを平文で送り、ファイル名や MIME は送らない。
        #expect(server.accepted["attachment_ids"] == .array([.string(attachmentID)]))
        #expect(server.requests.allSatisfy { !$0.contains("秘密.txt") && !$0.contains("text/plain") })
        _ = try receiver.store.processToDevice(server.delivered, keysQuery: dump(.object(try await server.snapshot())))
        let reader = MessageEncryption(store: { receiver.store }, api: server)
        let decrypted = reader.decrypt(row(server))
        let received = try #require(decrypted.payload?.attachments.first)
        #expect(received.name == "秘密.txt" && received.mimetype == "text/plain")
        // 表示用の行にして覚えると、暗号文から元の中身に戻せる。ハッシュが合わない暗号文は復号しない。
        let shown = MessageAttachment(decrypted: received)
        #expect(shown.content_type == "text/plain")
        #expect(EncryptedAttachmentFiles.shared.decrypt(encrypted.ciphertext, id: shown.id) == plaintext)
        var tampered = encrypted.ciphertext
        tampered[0] ^= 1
        #expect(EncryptedAttachmentFiles.shared.decrypt(tampered, id: shown.id) == nil)
    }

    @Test func 通知拡張は通知の暗号文の行を暗号ストアの読み取りだけで復号する() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        let secret = "通知の本文"
        try await encryption.send(channel: channel, id: messageID, parent: nil, body: secret)
        _ = try receiver.store.processToDevice(server.delivered, keysQuery: dump(.object(try await server.snapshot())))
        // 通知拡張が読む App Group の置き場に、受信端末の暗号ストアを写す（拡張は読み取りだけ）。
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.copyItem(at: receiver.directory, to: root.appendingPathComponent("store"))
        let storageKey = try #require(try receiver.files.keys.loadExisting())
        let before = try Data(contentsOf: root.appendingPathComponent("store/state.json"))

        let accepted = row(server)
        let cipherObject = try JSONSerialization.jsonObject(with: Data(accepted.ciphertext.utf8))
        func info(user: String = user, recipient: String = user, ciphertext: Any = cipherObject) -> [AnyHashable: Any] {
            ["user_id": user, "e2ee_v1": [
                "recipient_id": recipient,
                "message": ["channel_id": channel, "message_id": messageID, "parent_id": NSNull(), "user_id": user,
                            "message_kind": "message", "epoch": accepted.epoch, "revision": accepted.revision, "ciphertext": ciphertext]]]
        }
        #expect(NotificationE2ee.body(userInfo: info(), roots: [root], loadKey: { _ in storageKey }) == secret)
        // 読み取りだけ：状態は変わらない。
        #expect(try Data(contentsOf: root.appendingPathComponent("store/state.json")) == before)
        // 本文は 240 コードポイントまで。
        #expect((NotificationE2ee.body(userInfo: info(), roots: [root], loadKey: { _ in storageKey }) ?? "").unicodeScalars.count <= 240)
        // 保存鍵が違う、宛先が違う、置き場がない、暗号文が壊れているときは、復号しない。
        #expect(NotificationE2ee.body(userInfo: info(), roots: [root], loadKey: { _ in Data(repeating: 1, count: 32) }) == nil)
        #expect(NotificationE2ee.body(userInfo: info(), roots: [root], loadKey: { _ in nil }) == nil)
        #expect(NotificationE2ee.body(userInfo: info(recipient: "00000000-0000-4000-8000-0000000000ff"), roots: [root], loadKey: { _ in storageKey }) == nil)
        #expect(NotificationE2ee.body(userInfo: info(), roots: [], loadKey: { _ in storageKey }) == nil)
        #expect(NotificationE2ee.body(userInfo: info(ciphertext: ["sender_key": "x", "session_id": "y"]), roots: [root], loadKey: { _ in storageKey }) == nil)
        #expect(NotificationE2ee.body(userInfo: ["user_id": user], roots: [root], loadKey: { _ in storageKey }) == nil)
    }

    @Test func 送信した本文は送信端末でも再起動後に読める() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        try await encryption.send(channel: channel, id: messageID, parent: nil, body: "mine")
        try sender.restart()
        let reopened = MessageEncryption(store: { sender.store }, api: server)
        #expect(reopened.decrypt(row(server)).body == "mine")
    }

    @Test func 編集は同じepochで鍵を配り直さず次の版を送る() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        try await encryption.send(channel: channel, id: messageID, parent: nil, body: "first")
        let deliveredBefore = server.delivered.count
        let original = row(server)
        server.epoch = 2
        try await encryption.edit(original, body: "EDITED-SECRET")
        #expect(server.posts == 3)
        #expect(server.requests.allSatisfy { !$0.contains("EDITED-SECRET") })
        #expect(server.accepted["revision"] == .number(1))
        #expect(server.delivered.count == deliveredBefore) // 配布済みのセッションを再利用する。
        let reader = MessageEncryption(store: { sender.store }, api: server)
        #expect(reader.decrypt(row(server)).body == "EDITED-SECRET")
        // 古い版が来ても、最新の版を保つ。
        #expect(reader.decrypt(original).revision == .stale)
    }

    @Test func 鍵を配れなかったら本文を送らない() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        server.refuseDelivery = true
        server.staleOnce = false
        await #expect(throws: MessageEncryptionError.self) {
            try await encryption.send(channel: channel, id: messageID, parent: nil, body: "x")
        }
        #expect(server.posts == 0)
    }

    @Test func 鍵を配るあいだに端末が作り直されたら古い鍵のセッションで暗号化しない() async throws {
        let (server, sender, receiver, encryption, recoveryKey) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        server.staleOnce = false
        let oldKey = sender.store.curve25519Key
        // 最初の配布の応答を待つあいだに、端末の作り直しと再解錠が終わる。
        server.beforeToDevice = {
            try sender.store.replaceDevice()
            _ = try await sender.setup.unlock(.recovery(recoveryKey))
        }
        try await encryption.send(channel: channel, id: messageID, parent: nil, body: "race")
        // 本文は、新しい端末の鍵で、新しいセッションを配り直してから送る。
        let newKey = sender.store.curve25519Key
        #expect(newKey != oldKey)
        #expect(server.posts == 1)
        guard case .string(let ciphertext)? = server.accepted["ciphertext"], case .object(let envelope) = parse(ciphertext) else {
            Issue.record("ciphertext がない"); return
        }
        #expect(envelope["sender_key"] == .string(newKey))
        #expect(server.delivered.count == 2)
    }

    @Test func 配布先のページはクエリで渡し二ページ目にepochとafterを付ける() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        server.staleOnce = false
        server.pagedRecipients = true
        try await encryption.send(channel: channel, id: messageID, parent: nil, body: "ページ")
        // 1 ページ目はクエリなし、2 ページ目は `epoch` と `after`（パスに連結すると、`?` が符号化されて、実サーバーで失敗する）。
        let recipientQueries = server.queries.filter { !$0.isEmpty && $0["after"] != nil }
        #expect(recipientQueries.count == 1)
        #expect(recipientQueries.first?["epoch"] == "1")
        #expect(recipientQueries.first?["after"] == "page-2")
        #expect(server.posts == 1)
    }

    @Test func 鍵の配布の結果を読めなければ本文を送らない() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        server.omitUndelivered = true
        server.staleOnce = false
        await #expect(throws: MessageEncryptionError.self) {
            try await encryption.send(channel: channel, id: messageID, parent: nil, body: "x")
        }
        #expect(server.posts == 0)
    }

    @Test func epochが整数で表せなければ停止せず不正な応答にする() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        let broken = BrokenEpochServer(base: server)
        let guarded = MessageEncryption(store: { sender.store }, api: broken)
        await #expect(throws: (any Error).self) {
            try await guarded.send(channel: channel, id: messageID, parent: nil, body: "x")
        }
        _ = encryption
    }

    @Test func 解錠していなければ送れず復号は解錠を促す() async throws {
        let setupServer = FakeSetupServer(userID: user)
        let device = try Device(setupServer)
        defer { device.cleanUp() }
        let server = FakeMessageServer(setup: setupServer, sender: device, receiver: device)
        let encryption = MessageEncryption(store: { device.store }, api: server)
        await #expect(throws: MessageEncryptionError.sendNeedsUnlock) {
            try await encryption.send(channel: channel, id: messageID, parent: nil, body: "x")
        }
        let notReady = MessageEncryption(store: { nil }, api: server)
        #expect(notReady.decrypt(row(server)).notice == E2eeText.locked)
        await #expect(throws: MessageEncryptionError.deviceNotReady) {
            try await notReady.send(channel: channel, id: messageID, parent: nil, body: "x")
        }
    }

    @Test func 鍵がまだ届いていなければその旨を返す() async throws {
        let (server, sender, receiver, encryption, _) = try await makePair()
        defer { sender.cleanUp(); receiver.cleanUp() }
        try await encryption.send(channel: channel, id: messageID, parent: nil, body: "x")
        // to-device を処理していない受信端末。
        let reader = MessageEncryption(store: { receiver.store }, api: server)
        #expect(reader.decrypt(row(server)).notice == E2eeText.keyMissing)
    }
}

@MainActor
struct ConfirmedBodyTests {
    @Test func 古い版や中身の違う同じ版が届いても確認できた最新の本文を警告つきで出す() async throws {
        let setupServer = FakeSetupServer(userID: user)
        let sender = try Device(setupServer)
        defer { sender.cleanUp() }
        let start = try sender.setup.begin()
        try await sender.setup.completeSetup(start.session, reauth: "t")
        let server = PreviewE2eeServer(userID: user)
        let capture = CapturingRequests(user: user, store: { sender.store }, setupServer: setupServer)
        let encryption = MessageEncryption(store: { sender.store }, api: capture)
        let controller = E2eeController(
            manager: E2eeDeviceManager(api: server, files: sender.files, userID: user, observeToDevice: { _, _ in }),
            setup: E2eeSetup(api: server, userID: user, store: { sender.store }), encryption: encryption)

        func message(_ revision: UInt64) -> Message {
            Message(id: messageID, user_id: user, channel_id: channel, content: "", created_at: "2026-11-25T09:00:00Z",
                    ciphertext: CiphertextText(capture.ciphertext(revision)), epoch: ShapeInteger(capture.epoch(revision)),
                    revision: ShapeInteger(revision))
        }
        try await encryption.send(channel: channel, id: messageID, parent: nil, body: "最初")
        capture.record(revision: 0)
        let original = message(0)
        capture.epochValue = 2
        try await encryption.edit(EncryptedRow(channelID: channel, id: messageID, parentID: nil, userID: user, kind: "message",
                                               epoch: capture.epoch(0), revision: 0, ciphertext: capture.ciphertext(0)), body: "直した")
        capture.record(revision: 1)
        // 新しい版を読んだあと、古い版が届いても、新しい版の本文を警告つきで出す。
        #expect(controller.decrypt(message(1)).body == "直した")
        let stale = controller.decrypt(original)
        #expect(stale.body == "直した")
        #expect(stale.notice == E2eeText.revisionStale)
    }
}

/// 送った暗号文を版ごとに覚える、配布先が自分の端末だけの模擬の API。
@MainActor
final class CapturingRequests: E2eeRequesting {
    let user: String
    let store: () -> CryptoStore?
    let setupServer: FakeSetupServer
    var epochValue: UInt64 = 1
    private var accepted: [String: JSONValue] = [:]
    private var recorded: [UInt64: [String: JSONValue]] = [:]

    init(user: String, store: @escaping () -> CryptoStore?, setupServer: FakeSetupServer) {
        self.user = user; self.store = store; self.setupServer = setupServer
    }

    func record(revision: UInt64) { recorded[revision] = accepted }
    func ciphertext(_ revision: UInt64) -> String { if case .string(let text)? = recorded[revision]?["ciphertext"] { text } else { "" } }
    func epoch(_ revision: UInt64) -> UInt64 { if case .number(let value)? = recorded[revision]?["epoch"] { UInt64(value) } else { 0 } }

    func e2eeJSON(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?) async throws -> JSONValue {
        if path.hasPrefix("api/e2ee/channels/") {
            guard case .object(var page) = try JSONDecoder().decode(JSONValue.self, from: Data(try await setupServer.queryOwnKeys(userID: user).utf8)),
                  let store = store() else { return .object([:]) }
            page["membership_epoch"] = .number(Double(epochValue))
            page["user_ids"] = .array([.string(user)])
            page["device_keys"] = .object([user: .object([store.deviceID: try JSONDecoder().decode(JSONValue.self, from: Data(store.deviceKeys().utf8))])])
            return .object(page)
        }
        if path == "api/e2ee/to_device" { return .object(["undelivered": .array([])]) }
        if path.hasPrefix("api/messages") { accepted = body ?? [:] }
        return .object([:])
    }
}
