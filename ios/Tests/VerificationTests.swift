import ZooCrypto
import Foundation
import Testing
@testable import Zoo

@MainActor
private final class StubVerificationAPI: E2eeVerificationAPI {
    var queries = 0
    var snapshot = "{}"
    var claims: [String: JSONValue] = [:]
    var beforeSend: (() async throws -> Void)?
    var sends = 0
    func queryKeys(userID: String) async throws -> String { queries += 1; return snapshot }
    func queryKeys(userIDs: [String]) async throws -> String { queries += 1; return snapshot }
    func registerPin(_ pin: String) async throws -> Bool { true }
    func claim(_ oneTimeKeys: [String: JSONValue]) async throws -> [String: JSONValue] { claims }
    func sendToDevice(deviceID: String, messages: [String: JSONValue]) async throws -> Int {
        sends += 1
        try await beforeSend?()
        return 0
    }
    func saveAccepted(pin: String, signature: SignatureUpload, reauth: String) async throws {}
}

@MainActor
struct VerificationTests {
    @Test("検証要求の初回配信に失敗したら成功として残さない")
    func requestFailureRemovesEntry() async throws {
        let mineServer = FakeSetupServer(userID: user), mine = try Device(mineServer)
        defer { mine.cleanUp() }
        let start = try mine.setup.begin()
        try await mine.setup.completeSetup(start.session, reauth: "t")
        let otherID = "00000000-0000-4000-8000-000000000099"
        let (files, _, directory) = makeFiles()
        let other = try #require(try CryptoStore.open(files, userID: otherID)).store
        defer { other.close(); try? FileManager.default.removeItem(at: directory) }
        try other.markRegistered()
        let server = FakeSetupServer(userID: otherID)
        let setup = E2eeSetup(api: server, userID: otherID, store: { other })
        let otherStart = try setup.begin()
        try await setup.completeSetup(otherStart.session, reauth: "t")
        func value(_ text: String) throws -> JSONValue { try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8)) }
        var snapshot = try #require(value(try await mineServer.queryOwnKeys(userID: user)).object)
        let otherKeys = try #require(value(try await server.queryOwnKeys(userID: otherID)).object)
        for (key, item) in otherKeys {
            if let object = item.object { snapshot[key] = .object((snapshot[key]?.object ?? [:]).merging(object) { _, new in new }) }
            else if let array = item.array { snapshot[key] = .array((snapshot[key]?.array ?? []) + array) }
        }
        snapshot["device_keys"] = .object([otherID: .object([other.deviceID: try value(other.deviceKeys())])])
        snapshot["user_ids"] = .array([.string(user), .string(otherID)])
        let api = StubVerificationAPI()
        api.snapshot = String(decoding: try JSONEncoder().encode(JSONValue.object(snapshot)), as: UTF8.self)
        let keys = try #require(try other.prepareKeyUpload(oneTimeKeyCount: 0, hasUnusedFallbackKey: true))
        let first = try #require(keys.oneTimeKeys.first)
        api.claims = [otherID: .object([other.deviceID: .object([first.key: first.value])])]
        api.beforeSend = { throw URLError(.notConnectedToInternet) }
        let verification = E2eeVerification(store: { mine.store }, api: api)
        await #expect(throws: (any Error).self) { try await verification.request(userID: otherID) }
        #expect(api.sends == 1)
        #expect(verification.entries.isEmpty)
        api.beforeSend = nil
        _ = try await verification.request(userID: otherID)
        #expect(api.sends == 2 && verification.entries.count == 1)
    }

    @Test func 絵文字の表は番号0から63の64個で重複しない() {
        #expect(SasEmoji.table.count == 64)
        #expect(Set(SasEmoji.table.map(\.symbol)).count == 64)
        #expect(SasEmoji.symbol(0) == "🐶" && SasEmoji.symbol(63) == "📌")
        #expect(SasEmoji.symbol(64) == "?")
    }

    @Test func 解錠していない端末は検証を求められず相手の鍵も取らない() async {
        let api = StubVerificationAPI()
        let verification = E2eeVerification(store: { nil }, api: api)
        await #expect(throws: E2eeStoreError.sendNeedsUnlock) { try await verification.request(userID: UUID().uuidString) }
        #expect(api.queries == 0)
        #expect(verification.entries.isEmpty)
    }

    @Test func 解錠していない端末に届いた検証のメッセージは何も作らない() async throws {
        let api = StubVerificationAPI()
        let verification = E2eeVerification(store: { nil }, api: api)
        let message = #"{"sender_user_id":"u","sender_device":{},"type":"m.key.verification.request","content":{"transaction_id":"t"}}"#
        verification.handle([message])
        try await Task.sleep(for: .milliseconds(200))
        #expect(verification.entries.isEmpty)
        // 暗号ストアを使えないあいだは、捨てずに残す（Olm のメッセージは、2 回は復号できない）。形が壊れたものは、捨てる。
        #expect(verification.pendingMessageCount == 1)
        // 同じメッセージがもう一度届いても、重複して残さない。
        verification.handle([message])
        try await Task.sleep(for: .milliseconds(100))
        #expect(verification.pendingMessageCount == 1)
        verification.handle(["壊れたメッセージ"])
        try await Task.sleep(for: .milliseconds(200))
        #expect(verification.pendingMessageCount == 1)
    }
}
