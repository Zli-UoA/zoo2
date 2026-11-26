import ZooCrypto
import Foundation
import Testing
@testable import Zoo

@MainActor
private final class StubVerificationAPI: E2eeVerificationAPI {
    var queries = 0
    func queryKeys(userID: String) async throws -> String { queries += 1; return "{}" }
    func queryKeys(userIDs: [String]) async throws -> String { queries += 1; return "{}" }
    func registerPin(_ pin: String) async throws -> Bool { true }
    func claim(_ oneTimeKeys: [String: JSONValue]) async throws -> [String: JSONValue] { [:] }
    func sendToDevice(deviceID: String, messages: [String: JSONValue]) async throws -> Int { 0 }
    func saveAccepted(pin: String, signature: SignatureUpload, reauth: String) async throws {}
}

@MainActor
struct VerificationTests {
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
    }
}
