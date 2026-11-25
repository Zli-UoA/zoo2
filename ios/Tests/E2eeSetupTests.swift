import Foundation
import Testing
import ZooCrypto
@testable import Zoo

private func text(_ value: JSONValue) -> String { String(decoding: try! JSONEncoder().encode(value), as: UTF8.self) }
private func object(_ json: String) -> [String: JSONValue] {
    guard case .object(let value) = try! JSONDecoder().decode(JSONValue.self, from: Data(json.utf8)) else { fatalError() }
    return value
}

private struct Network: Error {}

/// 状態を持つ模擬サーバー（Android の `FakeSetupServer`、TUI の `tests/e2ee_setup.rs` と同じ）。
@MainActor
private final class FakeSetupServer: E2eeSetupAPI {
    let userID: String
    var defaultKeyID: String?
    var revision: UInt64 = 0
    var storageKey: JSONValue = .null
    var secrets: [String: JSONValue] = [:]
    var slots: [JSONValue] = []
    var keys: [String: JSONValue] = [:]
    var signatures: [JSONValue] = []
    var versions: [JSONValue] = []
    var reauthHeaders: [String] = []
    var usedSlots: [String] = []
    var failNext: Set<String> = []

    init(userID: String) { self.userID = userID }

    func reset() {
        defaultKeyID = nil; revision = 0; slots = []; keys = [:]; signatures = []; versions = []; failNext = []
    }

    private func fail(_ op: String) throws { if failNext.remove(op) != nil { throw Network() } }
    private func requireReauth(_ reauth: String?) throws {
        guard let reauth else { throw E2eeAPIError(status: 403, code: "E2EE_REAUTH_REQUIRED") }
        reauthHeaders.append(reauth)
    }
    private func checkDefaultKey(_ body: [String: JSONValue]) throws {
        guard case .string(let expected) = body["expected_default_key_id"] else { return }
        if expected != defaultKeyID { throw E2eeAPIError(status: 409, code: "E2EE_SSSS_CONFLICT") }
    }

    func secretStorage() async throws -> String {
        var response: [String: JSONValue] = [:]
        if let defaultKeyID {
            response["default_key_id"] = .string(defaultKeyID)
            response["keys"] = .object([defaultKeyID: storageKey])
            response["secrets"] = .object(secrets)
            response["secrets_revision"] = .number(Double(revision))
        } else {
            response["default_key_id"] = .null
            response["keys"] = .object([:]); response["secrets"] = .object([:]); response["secrets_revision"] = .null
        }
        response["slots"] = .array(slots)
        return text(.object(response))
    }

    func createSecretStorageKey(_ body: String, reauth: String?) async throws -> UInt64? {
        try requireReauth(reauth)
        try fail("secret_storage")
        let body = object(body)
        guard case .string(let keyID) = body["key_id"] else { fatalError() }
        if defaultKeyID != nil { throw E2eeAPIError(status: 409, code: "E2EE_SSSS_CONFLICT") }
        defaultKeyID = keyID
        storageKey = body["key"]!
        if case .object(let incoming) = body["secrets"] {
            secrets = incoming.mapValues { secret in
                guard case .object(var value) = secret else { return secret }
                value["key_id"] = .string(keyID)
                return .object(value)
            }
        }
        if case .array(let incoming) = body["slots"] { slots += incoming }
        revision += 1
        return revision
    }

    func addKeySlot(_ slot: String, reauth: String?) async throws {
        let slot = object(slot)
        let id = slot["id"]
        if !slots.contains(where: { if case .object(let existing) = $0 { existing["id"] == id } else { false } }) {
            slots.append(.object(slot))
        }
    }

    func markKeySlotUsed(_ slotID: String) async throws { usedSlots.append(slotID) }

    func queryOwnKeys(userID: String) async throws -> String {
        var response: [String: JSONValue] = ["device_keys": .object([:])]
        for (field, target) in [("master_key", "master_keys"), ("self_signing_key", "self_signing_keys"), ("user_signing_key", "user_signing_keys")] {
            response[target] = .object(keys[field].map { [self.userID: $0] } ?? [:])
        }
        response["signatures"] = .array(signatures)
        return text(.object(response))
    }

    func backupVersions() async throws -> String { text(.object(["versions": .array(versions)])) }

    func uploadCrossSigningKeys(_ body: String, reauth: String?) async throws {
        try requireReauth(reauth)
        let body = object(body)
        try checkDefaultKey(body)
        try fail("cross_signing")
        for field in ["master_key", "self_signing_key", "user_signing_key"] { if let value = body[field] { keys[field] = value } }
    }

    func uploadSignature(_ signature: String) async throws {
        try fail("signatures")
        let upload = object(signature)
        signatures.append(.object([
            "signer_user_id": .string(userID), "signer_key_id": upload["signer_key_id"]!,
            "target_user_id": upload["target_user_id"]!, "target_key_id": upload["target_key_id"]!,
            "signature": upload["signature"]!
        ]))
    }

    func createBackupVersion(_ body: String, reauth: String?) async throws {
        let body = object(body)
        try checkDefaultKey(body)
        try fail("versions")
        versions.append(.object(["version": .string(String(versions.count + 1)), "algorithm": body["algorithm"]!,
                                 "auth_data": body["auth_data"]!, "deleted": .bool(false)]))
    }
}

@MainActor
private final class Device {
    let server: FakeSetupServer
    let files: CryptoStoreFiles
    let directory: URL
    var store: CryptoStore
    var setup: E2eeSetup!

    init(_ server: FakeSetupServer) throws {
        self.server = server
        (files, _, directory) = makeFiles()
        store = try Device.open(files)
        setup = E2eeSetup(api: server, userID: user, store: { [unowned self] in self.store })
    }

    private static func open(_ files: CryptoStoreFiles) throws -> CryptoStore {
        let store = try #require(try CryptoStore.open(files, userID: user)).store
        if !store.isRegistered { try store.markRegistered() }
        return store
    }

    /// アプリを終了して開き直す（メモリーの鍵は消える）。
    func restart() throws {
        store.close()
        store = try Device.open(files)
    }

    func cleanUp() {
        store.close()
        try? FileManager.default.removeItem(at: directory)
    }
}

@MainActor
struct E2eeSetupTests {
    @Test func 初回設定は再認証のあと最後まで保存する() async throws {
        let server = FakeSetupServer(userID: user)
        let device = try Device(server)
        defer { device.cleanUp() }
        #expect(try await device.setup.loadStatus() == .notSetUp)
        let start = try device.setup.begin()
        #expect(start.recoveryKey.count == 48 + 11)
        // 再認証なしの保存は 403。同じ鍵のまま、再認証してやり直せる。
        await #expect(throws: E2eeSetupError.reauthRequired) { try await device.setup.completeSetup(start.session, reauth: nil) }
        #expect(server.defaultKeyID == nil)
        try await device.setup.completeSetup(start.session, reauth: "id-token")
        #expect(server.defaultKeyID == start.session.keyId())
        #expect(server.reauthHeaders == ["id-token", "id-token"])
        #expect(server.keys["master_key"] != nil)
        #expect(server.signatures.count == 1)
        #expect(server.versions.count == 1)
        #expect(device.store.hasOwnKeys)
        #expect(device.store.secretsRevision == 1)
        #expect(try await device.setup.loadStatus() == .unlocked)
    }

    @Test func どの保存の直前と直後で止めても同じ鍵のまま最後まで終わる() async throws {
        // SSSS の保存は、直前（サーバーに何も残らない）の失敗と、保存したあとに応答をなくした失敗を分けて確かめる。
        for stop in ["cross_signing", "signatures", "versions"] {
            let server = FakeSetupServer(userID: user)
            let device = try Device(server)
            defer { device.cleanUp() }
            let start = try device.setup.begin()
            server.failNext.insert(stop)
            await #expect(throws: Network.self) { try await device.setup.completeSetup(start.session, reauth: "t") }
            let master = server.keys["master_key"]
            // アプリを終了して開き直し、控えたリカバリーキーで解錠して続ける。
            try device.restart()
            #expect(try await device.setup.loadStatus() == .locked(.new), "\(stop)")
            let unlocked = try await device.setup.unlock(.recovery(start.recoveryKey))
            #expect(unlocked.needsReauth == (stop == "cross_signing"), "\(stop)")
            if unlocked.needsReauth {
                try await device.setup.saveRemaining(unlocked.session, secretsRevision: unlocked.secretsRevision, reauth: "t")
            }
            #expect(try await device.setup.loadStatus() == .unlocked, "\(stop)")
            if let master { #expect(server.keys["master_key"] == master, "\(stop)") }
            #expect(server.versions.count == 1, "\(stop)")
        }
    }

    @Test func SSSSの保存の応答をなくしても読み直して続きを保存する() async throws {
        let server = FakeSetupServer(userID: user)
        let device = try Device(server)
        defer { device.cleanUp() }
        let start = try device.setup.begin()
        // 保存は済んだが応答が届かなかった状態を作る（同じ本文を直接保存する）。
        _ = try await server.createSecretStorageKey(try start.session.secretStorageRequest(), reauth: "t")
        try await device.setup.completeSetup(start.session, reauth: "t")
        #expect(try await device.setup.loadStatus() == .unlocked)
        #expect(server.versions.count == 1)
    }

    @Test func 別の端末が先に初回設定をしていたら手元の鍵を捨てる() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let second = try Device(server)
        defer { first.cleanUp(); second.cleanUp() }
        let mine = try first.setup.begin()
        let theirs = try second.setup.begin()
        try await second.setup.completeSetup(theirs.session, reauth: "t")
        await #expect(throws: E2eeSetupError.conflict) { try await first.setup.completeSetup(mine.session, reauth: "t") }
        #expect(server.defaultKeyID == theirs.session.keyId())
    }

    @Test func 別の端末はリカバリーキーで解錠して自分に署名する() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let second = try Device(server)
        defer { first.cleanUp(); second.cleanUp() }
        let start = try first.setup.begin()
        try await first.setup.completeSetup(start.session, reauth: "t")
        #expect(try await second.setup.loadStatus() == .locked(.new))
        let unlocked = try await second.setup.unlock(.recovery(start.recoveryKey))
        #expect(!unlocked.needsReauth)
        #expect(second.store.hasOwnKeys)
        #expect(server.signatures.count == 2)
        #expect(try await second.setup.loadStatus() == .unlocked)
        #expect(server.usedSlots.count == 1)
    }

    @Test func 間違ったリカバリーキーでは解錠できない() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let second = try Device(server)
        defer { first.cleanUp(); second.cleanUp() }
        let start = try first.setup.begin()
        try await first.setup.completeSetup(start.session, reauth: "t")
        let other = try first.setup.begin()
        await #expect(throws: (any Error).self) { try await second.setup.unlock(.recovery(other.recoveryKey)) }
        #expect(!second.store.hasOwnKeys)
    }

    @Test func SSSSがなければ解錠できない() async throws {
        let server = FakeSetupServer(userID: user)
        let device = try Device(server)
        defer { device.cleanUp() }
        await #expect(throws: E2eeSetupError.noSecretStorage) { try await device.setup.unlock(.recovery("x")) }
    }

    @Test func secrets_revisionが変わったら解錠し直しを求める() async throws {
        let server = FakeSetupServer(userID: user)
        let device = try Device(server)
        defer { device.cleanUp() }
        let start = try device.setup.begin()
        try await device.setup.completeSetup(start.session, reauth: "t")
        server.revision += 1
        #expect(try await device.setup.loadStatus() == .locked(.mismatch))
        // 解錠し直すまで、暗号ストアの鍵を使う処理を止める（記録を消す）。
        #expect(device.store.secretsRevision == nil)
    }
}
