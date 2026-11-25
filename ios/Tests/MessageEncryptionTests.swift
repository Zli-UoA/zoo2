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

    func e2eeJSON(_ path: String, method: String, body: [String: JSONValue]?) async throws -> JSONValue {
        requests.append(body.map { dump(.object($0)) } ?? "")
        switch path.split(separator: "?").first.map(String.init) ?? path {
        case "api/e2ee/channels/\(channel)/recipients":
            return .object(try await snapshot())
        case "api/e2ee/keys/claim":
            let key = try #require(try receiver.store.prepareKeyUpload(oneTimeKeyCount: 0, hasUnusedFallbackKey: true)).oneTimeKeys.first!
            return .object(["one_time_keys": .object([user: .object([receiver.store.deviceID: .object([key.key: key.value])])])])
        case "api/e2ee/to_device":
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
    func e2eeJSON(_ path: String, method: String, body: [String: JSONValue]?) async throws -> JSONValue {
        guard path.hasPrefix("api/e2ee/channels/") else { return try await base.e2eeJSON(path, method: method, body: body) }
        guard case .object(var page) = try await base.e2eeJSON(path, method: method, body: body) else { return .object([:]) }
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
