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
final class FakeSetupServer: E2eeSetupAPI {
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
    /// 版の作成にも、直前の再認証を求める（過去の版があるユーザーの版の作り直し）。
    var requireReauthForVersions = false

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

    /// 0 より大きい間、`secret_storage` の取得を通信エラーにして、1 つずつ減らす。
    var failSecretStorage = 0
    /// 1 回目の取得のあと、通信エラーにする。
    var failSecretStorageAfterFirst = false
    private var secretStorageCalls = 0

    func secretStorage() async throws -> String {
        if failSecretStorageAfterFirst { if secretStorageCalls >= 1 { throw URLError(.notConnectedToInternet) }; secretStorageCalls += 1 }
        if failSecretStorage > 0 { failSecretStorage -= 1; throw URLError(.notConnectedToInternet) }
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
        if let current = defaultKeyID {
            // E2EE のリセット：サーバーの今の既定の鍵と版が前提のときだけ、作り直す（鍵スロットも置き換える）。
            guard case .string(let expected) = body["expected_default_key_id"], expected == current,
                  case .number(let expectedRevision)? = body["expected_secrets_revision"], UInt64(expectedRevision) == revision else {
                throw E2eeAPIError(status: 409, code: "E2EE_SSSS_CONFLICT")
            }
            slots = []
        }
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

    // MARK: 鍵バックアップ

    /// 今の版に入れる `upload_key`（証明の確認用。なければ付けない）。
    var uploadKey: JSONValue?
    /// 送られたセッション（`(channel_id, sender_key, session_id)` ごとに 1 つ）と、`X-Zoo-Backup-Proof` の値の記録。
    var backupRows: [String: JSONValue] = [:]
    var backupProofs: [String?] = []
    var backupUploads: [Int] = []
    var backupConflict: String?

    func currentBackupVersion() async throws -> String? {
        guard case .object(var version)? = versions.last else { return nil }
        if let uploadKey { version["upload_key"] = uploadKey }
        return text(.object(version))
    }

    func backupKeys(version: String, after: String?) async throws -> String {
        if let backupPage { return backupPage(after) }
        return text(.object(["sessions": .array(Array(backupRows.values)), "next": .null]))
    }

    /// 取り込みのページの応答を差し替える（不正なページと、同じカーソルの繰り返しを試す）。
    var backupPage: ((String?) -> String)?

    func uploadBackupKeys(version: String, secretsRevision: UInt64, body: String, proof: String?) async throws {
        if let backupConflict { throw E2eeAPIError(status: 409, code: backupConflict) }
        guard case .object(let last)? = versions.last, last["version"] == .string(version), secretsRevision == revision else {
            throw E2eeAPIError(status: 409, code: "E2EE_WRONG_BACKUP_VERSION")
        }
        backupProofs.append(proof)
        guard case .array(let sessions)? = object(body)["sessions"] else { return }
        backupUploads.append(sessions.count)
        for session in sessions {
            guard case .object(let row) = session, case .string(let channel)? = row["channel_id"],
                  case .string(let sender)? = row["sender_key"], case .string(let id)? = row["session_id"] else { continue }
            backupRows["\(channel)|\(sender)|\(id)"] = session
        }
    }

    func createBackupVersion(_ body: String, reauth: String?) async throws {
        let body = object(body)
        if requireReauthForVersions { try requireReauth(reauth) }
        try checkDefaultKey(body)
        try fail("versions")
        versions.append(.object(["version": .string(String(versions.count + 1)), "algorithm": body["algorithm"]!,
                                 "auth_data": body["auth_data"]!, "deleted": .bool(false)]))
    }
}

@MainActor
final class Device {
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

    @Test func 版の作り直しに再認証が要るときは解錠を失敗にせず再認証を求める() async throws {
        let server = FakeSetupServer(userID: user)
        let device = try Device(server)
        defer { device.cleanUp() }
        let start = try device.setup.begin()
        server.failNext.insert("versions")
        await #expect(throws: Network.self) { try await device.setup.completeSetup(start.session, reauth: "t") }
        try device.restart()
        server.requireReauthForVersions = true
        let unlocked = try await device.setup.unlock(.recovery(start.recoveryKey))
        #expect(unlocked.needsReauth)
        #expect(!device.store.hasOwnKeys)
        try await device.setup.saveRemaining(unlocked.session, secretsRevision: unlocked.secretsRevision, reauth: "t")
        #expect(try await device.setup.loadStatus() == .unlocked)
        #expect(server.versions.count == 1)
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

    @Test func E2EEのリセットはサーバーの今の鍵と版を前提に鍵を作り直して保存する() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let start = try first.setup.begin()
        try await first.setup.completeSetup(start.session, reauth: "t")
        let oldKeyID = server.defaultKeyID
        let oldMaster = server.keys["master_key"]
        let second = try Device(server)
        defer { first.cleanUp(); second.cleanUp() }

        let (reset, expectation) = try await second.setup.beginReset()
        #expect(expectation.defaultKeyID == oldKeyID && expectation.secretsRevision == 1)
        // 再認証なしでは保存できない（何も変わらない）。
        await #expect(throws: E2eeSetupError.reauthRequired) {
            try await second.setup.completeReset(reset.session, expectation: expectation, reauth: nil)
        }
        #expect(server.defaultKeyID == oldKeyID)
        try await second.setup.completeReset(reset.session, expectation: expectation, reauth: "t")
        // 新しい SSSS 鍵と、新しいマスター鍵、新しい版（今の版）になる。
        #expect(server.defaultKeyID == reset.session.keyId() && server.defaultKeyID != oldKeyID)
        #expect(server.keys["master_key"] != oldMaster)
        #expect(server.versions.count == 2)
        #expect(server.slots.count == 1)
        #expect(second.store.hasOwnKeys && second.store.secretsRevision == server.revision)
        // 古い端末は、サーバーの鍵が手元の鍵と違うので、解錠し直しを求められる。
        #expect(try await first.setup.loadStatus() == .locked(.mismatch))
        // 新しいリカバリーキーで、別の端末が解錠できる。
        let third = try Device(server)
        defer { third.cleanUp() }
        _ = try await third.setup.unlock(.recovery(reset.recoveryKey))
        #expect(third.store.hasOwnKeys)
    }

    @Test func E2EEのリセットをSSSSの保存後に止めても新しいリカバリーキーで解錠してリセットの続きから終える() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let start = try first.setup.begin()
        try await first.setup.completeSetup(start.session, reauth: "t")
        let oldMaster = server.keys["master_key"]
        let second = try Device(server)
        defer { first.cleanUp(); second.cleanUp() }
        let (reset, expectation) = try await second.setup.beginReset()
        // SSSS は新しい鍵で保存できたが、クロス署名鍵の保存で止まる（置き換える前のマスター鍵が残る）。
        server.failNext.insert("cross_signing")
        await #expect(throws: (any Error).self) {
            try await second.setup.completeReset(reset.session, expectation: expectation, reauth: "t")
        }
        #expect(server.defaultKeyID == reset.session.keyId() && server.keys["master_key"] == oldMaster)
        #expect(second.store.resetKeyID == reset.session.keyId())
        // アプリを開き直して、新しいリカバリーキーで解錠する。リセットの続きなので、置き換える前の鍵が前提の要求で終わる。
        try second.restart()
        let unlocked = try await second.setup.unlock(.recovery(reset.recoveryKey))
        #expect(unlocked.needsReauth)
        try await second.setup.saveRemaining(unlocked.session, secretsRevision: unlocked.secretsRevision, reauth: "t")
        #expect(server.keys["master_key"] != oldMaster && server.versions.count == 2)
        #expect(second.store.resetKeyID == nil && second.store.hasOwnKeys)
    }

    @Test func SSSSが書き換えられていたら解錠できたとは返さず解錠し直しを求める() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let start = try first.setup.begin()
        try await first.setup.completeSetup(start.session, reauth: "t")
        let second = try Device(server)
        defer { first.cleanUp(); second.cleanUp() }
        _ = try await second.setup.unlock(.recovery(start.recoveryKey))
        // 鍵バックアップに送れていない受信用セッションを 1 つ作る（自分が送った本文のもの）。
        let capture = CapturingRequests(user: user, store: { second.store }, setupServer: server)
        try await MessageEncryption(store: { second.store }, api: capture).send(
            channel: "00000000-0000-4000-8000-0000000000c1", id: "00000000-0000-4000-8000-0000000000d1", parent: nil, body: "x")
        server.backupConflict = "E2EE_SSSS_CONFLICT"
        // 解錠の保存の末尾で、鍵バックアップの送信が SSSS の競合になる。解錠できたとは返さない。
        await #expect(throws: E2eeSetupError.secretsChanged) { _ = try await second.setup.unlock(.recovery(start.recoveryKey)) }
    }

    @Test func リセットの印はSSSSの保存の要求を送る前に残す() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let start = try first.setup.begin()
        try await first.setup.completeSetup(start.session, reauth: "t")
        let second = try Device(server)
        defer { first.cleanUp(); second.cleanUp() }
        let (reset, expectation) = try await second.setup.beginReset()
        // SSSS の保存の要求が通らなくても（応答をなくす、アプリが終わる場合も同じ）、印は残っている。
        server.failNext.insert("secret_storage")
        await #expect(throws: (any Error).self) {
            try await second.setup.completeReset(reset.session, expectation: expectation, reauth: "t")
        }
        #expect(second.store.resetKeyID == reset.session.keyId())
    }

    @Test func E2EEのリセットは前提の版が古ければ別の端末が先にリセットしたとして止まる() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let start = try first.setup.begin()
        try await first.setup.completeSetup(start.session, reauth: "t")
        let second = try Device(server)
        let other = try Device(server)
        defer { first.cleanUp(); second.cleanUp(); other.cleanUp() }
        let (mine, expectation) = try await second.setup.beginReset()
        // 読んだあとに、別の端末が先にリセットを終える。
        let (theirs, theirExpectation) = try await other.setup.beginReset()
        try await other.setup.completeReset(theirs.session, expectation: theirExpectation, reauth: "t")
        await #expect(throws: E2eeSetupError.conflict) {
            try await second.setup.completeReset(mine.session, expectation: expectation, reauth: "t")
        }
        #expect(server.defaultKeyID == theirs.session.keyId())
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

struct PasskeyCodingTests {
    @Test func base64URLはパディングなしで往復する() throws {
        let data = Data([0xfb, 0xff, 0xfe, 0x00, 0x10])
        let text = PasskeyCoding.base64URL(data)
        #expect(!text.contains("=") && !text.contains("+") && !text.contains("/"))
        #expect(PasskeyCoding.decodeBase64URL(text) == data)
        #expect(PasskeyCoding.decodeBase64URL(text + "==") == data)
        #expect(PasskeyCoding.decodeBase64URL("***") == nil)
    }

    @Test func PRFの出力は32バイトだけを受け付ける() throws {
        #expect(try PasskeyCoding.checkedOutput(Data(count: 32)).count == 32)
        #expect(throws: PasskeyError.prfUnsupported) { try PasskeyCoding.checkedOutput(nil) }
        #expect(throws: PasskeyError.prfUnsupported) { try PasskeyCoding.checkedOutput(Data(count: 31)) }
    }
}

/// 解錠の手段にパスキーを渡す経路（PRF の出力でスロットを開ける）。
@MainActor
private final class FakePasskey: PasskeyClient {
    var output = Data(repeating: 7, count: 32)
    var salts: [Data] = []
    var credential = "Y3JlZC0x"
    func prfForNewSlot(rpID: String, salt: Data) async throws -> PasskeyResult {
        salts.append(salt)
        return PasskeyResult(credentialID: credential, prfOutput: output)
    }
    func prfForUnlock(rpID: String, slots: [PasskeySlot]) async throws -> PasskeyResult {
        PasskeyResult(credentialID: slots.first?.credentialId ?? credential, prfOutput: output)
    }
}

@MainActor
struct PasskeyUnlockTests {
    @Test func パスキーのスロットを足した初回設定を同じPRFで解錠できる() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let second = try Device(server)
        defer { first.cleanUp(); second.cleanUp() }
        let passkey = FakePasskey()
        let start = try first.setup.begin()
        try await first.setup.addSetupPasskey(start.session, client: passkey, rpID: "account.zli.works", label: "iPhone")
        try await first.setup.completeSetup(start.session, reauth: "t")
        let unlocked = try await second.setup.unlock(.passkey(passkey, rpID: "account.zli.works"))
        #expect(!unlocked.needsReauth)
        #expect(second.store.hasOwnKeys)
        #expect(try await second.setup.loadStatus() == .unlocked)
    }

    @Test func 違うPRFの出力では解錠できず自分の鍵を保存しない() async throws {
        let server = FakeSetupServer(userID: user)
        let first = try Device(server)
        let second = try Device(server)
        defer { first.cleanUp(); second.cleanUp() }
        let passkey = FakePasskey()
        let start = try first.setup.begin()
        try await first.setup.addSetupPasskey(start.session, client: passkey, rpID: "account.zli.works", label: nil)
        try await first.setup.completeSetup(start.session, reauth: "t")
        passkey.output = Data(repeating: 9, count: 32)
        await #expect(throws: (any Error).self) { try await second.setup.unlock(.passkey(passkey, rpID: "account.zli.works")) }
        #expect(!second.store.hasOwnKeys)
    }
}

struct ReauthTests {
    private func request() throws -> ReauthRequest {
        try ReauthRequest.create(authorizationEndpoint: "https://account.zli.works/api/auth/oauth2/authorize",
                                 clientID: "client-1", resource: "https://zli.chat/api",
                                 state: "s1", nonce: "n1", verifier: "v".padding(toLength: 43, withPad: "v", startingAt: 0))
    }

    @Test func 認可リクエストはmax_age0とPKCEとstateとnonceを持ちoffline_accessを持たない() throws {
        let items = Dictionary(uniqueKeysWithValues: URLComponents(url: try request().url, resolvingAgainstBaseURL: false)!
            .queryItems!.map { ($0.name, $0.value ?? "") })
        #expect(items["max_age"] == "0")
        #expect(items["code_challenge_method"] == "S256")
        #expect(items["scope"] == "openid profile")
        #expect(items["state"] == "s1" && items["nonce"] == "n1")
        #expect(items["redirect_uri"] == "works.zli.zoo:/reauth-callback")
        #expect(items["code_challenge"] == ReauthRequest.codeChallenge(String(repeating: "v", count: 43)))
    }

    @Test func 認可エンドポイントはHTTPSだけ() {
        #expect(throws: (any Error).self) {
            try ReauthRequest.create(authorizationEndpoint: "http://account.zli.works/authorize", clientID: "c", resource: "")
        }
    }

    @Test func PKCEのS256はRFC7636の例と一致する() {
        #expect(ReauthRequest.codeChallenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")
            == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
    }

    @Test func コールバックはstateが合うときだけ受け取る() throws {
        let request = try request()
        #expect(ReauthCallback.parse(URL(string: "works.zli.zoo:/reauth-callback?code=abc&state=s1")!, request: request) == .code("abc"))
        #expect(ReauthCallback.parse(URL(string: "works.zli.zoo:/reauth-callback?code=abc&state=other")!, request: request) == nil)
        #expect(ReauthCallback.parse(URL(string: "works.zli.zoo:/elsewhere?code=abc&state=s1")!, request: request) == nil)
        #expect(ReauthCallback.parse(URL(string: "works.zli.zoo:/reauth-callback?error=access_denied&state=s1")!, request: request)
            == .error("access_denied"))
        #expect(ReauthCallback.parse(URL(string: "works.zli.zoo:/reauth-callback?state=s1")!, request: request)
            == .error("認可コードがありません"))
    }

    private func token(_ claims: [String: Any]) -> String {
        let payload = PasskeyCoding.base64URL(try! JSONSerialization.data(withJSONObject: claims))
        return "e30.\(payload).sig"
    }

    @Test func IDトークンのiss_aud_exp_nonceを確かめる() throws {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let good: [String: Any] = ["iss": "https://a/", "aud": ["x", "client-1"], "exp": 1_000_100, "nonce": "n1"]
        try validateIDToken(token(good), issuer: "https://a", clientID: "client-1", nonce: "n1", now: now)
        for (change, message) in [("iss", "iss"), ("aud", "aud"), ("exp", "期限"), ("nonce", "nonce")] {
            var claims = good
            claims[change] = change == "exp" ? 1_000_000 - 61 : "bad"
            #expect(throws: ReauthError.self, "\(message)") {
                try validateIDToken(token(claims), issuer: "https://a", clientID: "client-1", nonce: "n1", now: now)
            }
        }
        #expect(throws: ReauthError.self) { try validateIDToken("not-a-jwt", issuer: "i", clientID: "c", nonce: "n") }
    }
}

