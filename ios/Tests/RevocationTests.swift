import Foundation
import Testing
import ZooCrypto
@testable import Zoo

private func text(_ value: JSONValue) -> String { String(decoding: try! JSONEncoder().encode(value), as: UTF8.self) }
private func parse(_ json: String) -> [String: JSONValue] {
    guard case .object(let value) = try! JSONDecoder().decode(JSONValue.self, from: Data(json.utf8)) else { fatalError() }
    return value
}

/// 失効の API を模した、状態を持つサーバー（初回設定のあとの状態から始める）。
@MainActor
private final class RevocationServer: E2eeRevocationAPI {
    var storage: [String: JSONValue]
    var keys: [String: JSONValue]
    var signatures: [JSONValue]
    var versions: [JSONValue]
    var deleted: [String] = []
    var requests: [String] = []
    var reauthSeen: [String?] = []
    var requireReauth = true
    /// 次の 1 回だけ、このパスの要求を通信エラーにする（途中で止めて再開するテスト）。
    var failOnce: String?

    init(from server: FakeSetupServer, userID: String) async throws {
        storage = parse(try await server.secretStorage())
        keys = parse(try await server.queryOwnKeys(userID: userID))
        signatures = keys["signatures"]?.array ?? []
        versions = parse(try await server.backupVersions())["versions"]?.array ?? []
    }

    func call(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?, reauth: String?,
              headers: [String: String]) async throws -> JSONValue {
        requests.append("\(method) \(path)")
        let guardedWrite = method != "GET" && method != "DELETE" && path != "api/e2ee/keys/query"
        if guardedWrite, requireReauth, reauth == nil, path != "api/e2ee/key_backup/keys" { throw E2eeAPIError(status: 403, code: "E2EE_REAUTH_REQUIRED") }
        reauthSeen.append(reauth)
        if failOnce == path { failOnce = nil; throw E2eeAPIError(status: 500, code: nil) }
        switch (method, path) {
        case ("POST", "api/e2ee/secret_storage/keys"):
            // 全範囲：サーバーの今の既定の鍵と版が前提のときだけ、新しい SSSS 鍵に作り直す（鍵スロットも置き換える）。
            guard body?["expected_default_key_id"]?.string == storage["default_key_id"]?.string,
                  body?["expected_secrets_revision"]?.uint == storage["secrets_revision"]?.uint else {
                throw E2eeAPIError(status: 409, code: "E2EE_SSSS_CONFLICT")
            }
            let keyID = body?["key_id"] ?? .null
            var secrets = body?["secrets"]?.object ?? [:]
            for (name, secret) in secrets {
                guard case .object(var value) = secret else { continue }
                value["key_id"] = keyID
                secrets[name] = .object(value)
            }
            let revision = (storage["secrets_revision"]?.uint ?? 0) + 1
            storage = ["default_key_id": keyID, "keys": .object([keyID.string ?? "": body?["key"] ?? .null]),
                       "secrets": .object(secrets), "secrets_revision": .number(Double(revision)),
                       "slots": body?["slots"] ?? .array([])]
            return .object(["secrets_revision": .number(Double(revision))])
        case ("GET", "api/e2ee/secret_storage"): return .object(storage)
        case ("POST", "api/e2ee/secret_storage/secrets"):
            let revision = (storage["secrets_revision"]?.uint ?? 0) + 1
            var secrets = storage["secrets"]?.object ?? [:]
            for (name, secret) in body?["secrets"]?.object ?? [:] {
                guard case .object(var value) = secret else { continue }
                value["key_id"] = storage["default_key_id"]
                secrets[name] = .object(value)
            }
            storage["secrets"] = .object(secrets)
            storage["secrets_revision"] = .number(Double(revision))
            return .object(["secrets_revision": .number(Double(revision))])
        case ("POST", "api/e2ee/cross_signing/keys"):
            for (field, target) in [("master_key", "master_keys"), ("self_signing_key", "self_signing_keys"), ("user_signing_key", "user_signing_keys")] {
                if let key = body?[field] { keys[target] = .object([storage["user_id"]?.string ?? userOf(keys): key]) }
            }
            return .null
        case ("POST", "api/e2ee/cross_signing/signatures"):
            let owner = userOf(keys)
            for case .object(let upload) in body?["signatures"]?.array ?? [] {
                signatures.append(.object(["signer_user_id": .string(owner), "signer_key_id": upload["signer_key_id"]!,
                                           "target_user_id": upload["target_user_id"]!, "target_key_id": upload["target_key_id"]!,
                                           "signature": upload["signature"]!]))
            }
            return .null
        case ("POST", "api/e2ee/keys/query"):
            var response = keys
            response["signatures"] = .array(signatures)
            return .object(response)
        case ("GET", "api/e2ee/key_backup/versions"): return .object(["versions": .array(versions)])
        case ("POST", "api/e2ee/key_backup/versions"):
            let number = String(versions.count + 1)
            versions.append(.object(["version": .string(number), "algorithm": body?["algorithm"] ?? .null,
                                     "auth_data": body?["auth_data"] ?? .null, "deleted": .bool(false)]))
            return .object(["version": .string(number)])
        case ("GET", "api/e2ee/key_backup/versions/current"):
            guard let current = versions.last(where: { $0.object?["deleted"] != .bool(true) }) else { throw E2eeAPIError(status: 404, code: nil) }
            return current
        case ("GET", "api/e2ee/tofu_pins"): return .object(["pins": .array([]), "next": .null])
        case ("GET", "api/e2ee/key_backup/keys"): return .object(["sessions": .array([]), "next": .null])
        default:
            if method == "DELETE", path.hasPrefix("api/e2ee/key_backup/versions/") {
                let version = String(path.dropFirst("api/e2ee/key_backup/versions/".count))
                deleted.append(version)
                versions = versions.map { value in
                    guard case .object(var object) = value, object["version"]?.string == version else { return value }
                    object["deleted"] = .bool(true)
                    return .object(object)
                }
                return .null
            }
            if method == "GET", path.hasPrefix("api/e2ee/key_backup/versions/") {
                let version = String(path.dropFirst("api/e2ee/key_backup/versions/".count))
                return versions.first { $0.object?["version"]?.string == version } ?? .null
            }
            throw E2eeAPIError(status: 404, code: nil)
        }
    }

    private func userOf(_ keys: [String: JSONValue]) -> String { keys["master_keys"]?.object?.keys.first ?? "" }
}

@MainActor
struct RevocationTests {
    /// 初回設定を済ませた端末と、その状態から始まる模擬サーバーと、開けた SSSS。
    private func prepared() async throws -> (Device, RevocationServer, UnlockedSecretStorage, String) {
        let fake = FakeSetupServer(userID: user)
        let device = try Device(fake)
        let start = try device.setup.begin()
        try await device.setup.completeSetup(start.session, reauth: "t")
        let server = try await RevocationServer(from: fake, userID: user)
        let storage = try #require(try parseSecretStorage(userId: user, response: try await fake.secretStorage()))
        let unlocked = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: start.recoveryKey))
        return (device, server, unlocked, start.recoveryKey)
    }

    @Test func 端末だけの失効は鍵を作り直して保存し記録を消す() async throws {
        let (device, server, unlocked, firstRecoveryKey) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        let oldMaster = server.keys["master_keys"]
        let started = try await revocation.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        #expect(started.stage == "prepared")
        // 始めた時点で、進み具合が暗号ストアにある（アプリを閉じても続けられる）。
        #expect(try revocation.savedSummary()?.revocation_id == started.revocation_id)
        // 終わっていない失効があるうちは、新しい失効を始めない。
        await #expect(throws: RevocationError.pending) {
            _ = try await revocation.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        }

        // 再認証なしでは、保存の最初の要求が 403 になる（何も保存されない）。
        await #expect(throws: E2eeSetupError.reauthRequired) { try await revocation.run(reauth: nil) }
        #expect(try revocation.savedSummary()?.stage == "prepared")

        let finished = try await revocation.run(reauth: "token")
        #expect(finished.stage == "verified")
        #expect(try revocation.savedSummary() == nil)
        // 端末だけの失効は、マスター鍵を残し、SSSS の秘密を置き換え（版が進む）、新しい版を作って古い版を消す。
        #expect(server.keys["master_keys"] == oldMaster)
        #expect(server.storage["secrets_revision"]?.uint == 2)
        #expect(server.versions.count == 2)
        #expect(server.deleted == ["1"])
        #expect(server.requests.contains("POST api/e2ee/cross_signing/signatures"))
        // 暗号ストアの自分の鍵は、保存した新しい鍵にそろい、新しい SSSS の版を覚えている。
        #expect(device.store.secretsRevision == 2)
    }

    @Test func 全範囲の失効は新しいSSSS鍵とマスター鍵を作り直し古いリカバリーキーを使えなくする() async throws {
        let (device, server, unlocked, firstRecoveryKey) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        let oldKeyID = server.storage["default_key_id"]?.string
        let oldMaster = server.keys["master_keys"]
        _ = try await revocation.begin(scope: .full, deletedDevices: [], unlocked: unlocked)
        // 鍵スロットがないと、保存の要求を作れない。
        await #expect(throws: (any Error).self) { try await revocation.run(reauth: "token") }
        let recoveryKey = try revocation.newRecoveryKey()

        let finished = try await revocation.run(reauth: "token")
        #expect(finished.stage == "verified")
        #expect(try revocation.savedSummary() == nil)
        // 新しい SSSS 鍵（版が進む）、新しいマスター鍵、新しい版（古い版は消える）。
        #expect(server.storage["default_key_id"]?.string != oldKeyID)
        #expect(server.storage["secrets_revision"]?.uint == 2)
        #expect(server.keys["master_keys"] != oldMaster)
        #expect(server.storage["slots"]?.array?.count == 1)
        #expect(server.versions.count == 2 && server.deleted == ["1"])
        #expect(device.store.secretsRevision == 2)
        // 新しいリカバリーキーだけが、新しい SSSS を開ける。
        let storage = try #require(try parseSecretStorage(userId: user, response: text(.object(server.storage))))
        _ = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: recoveryKey))
        let old = try RecoveryKey.parse(input: firstRecoveryKey)
        #expect(throws: (any Error).self) { _ = try storage.unlockWithRecoveryKey(recoveryKey: old) }
    }

    @Test func 全範囲をSSSSの保存の前に止めたら最初からやり直す() async throws {
        let (device, server, unlocked, _) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await revocation.begin(scope: .full, deletedDevices: [], unlocked: unlocked)
        // アプリを閉じて開き直した（メモリーの秘密は消え、進み具合だけが暗号ストアにある）。
        let reopened = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        #expect(try reopened.savedSummary()?.stage == "prepared")
        #expect(throws: ZooCryptoError.self) { _ = try reopened.resume(unlocked: unlocked) }
        // 捨てれば、新しい失効を始められる。
        try reopened.abandon()
        #expect(try reopened.savedSummary() == nil)
    }

    @Test func 全範囲をクロス署名鍵の保存で止めても新しいSSSSを開け直して続きから終える() async throws {
        let (device, server, unlocked, _) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await revocation.begin(scope: .full, deletedDevices: [], unlocked: unlocked)
        let recoveryKey = try revocation.newRecoveryKey()
        server.failOnce = "api/e2ee/cross_signing/keys"
        await #expect(throws: (any Error).self) { try await revocation.run(reauth: "token") }
        // SSSS は新しい鍵で保存済みで、進み具合は暗号ストアにある。
        #expect(try revocation.savedSummary()?.stage == "secret_storage_saved")

        // アプリを開き直して、新しいリカバリーキーで SSSS を開け直し、続きから終える。
        let reopened = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        let storage = try #require(try parseSecretStorage(userId: user, response: text(.object(server.storage))))
        let again = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: recoveryKey))
        let resumed = try reopened.resume(unlocked: again)
        #expect(resumed.reached("secret_storage_saved") && !resumed.reached("cross_signing_saved"))
        let finished = try await reopened.run(reauth: "token")
        #expect(finished.stage == "verified")
        #expect(try reopened.savedSummary() == nil)
        #expect(server.versions.count == 2)
    }
}

@MainActor
struct KeySlotsTests {
    /// 鍵スロットの API を模した、状態を持つサーバー。
    private final class Server: E2eeRevocationAPI {
        var storage: JSONValue
        var reauthSeen: [String?] = []
        var failReauthOnce = false
        init(storage: JSONValue) { self.storage = storage }

        func call(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?, reauth: String?,
                  headers: [String: String]) async throws -> JSONValue {
            var object = storage.object ?? [:]
            var slots = object["slots"]?.array ?? []
            switch (method, path) {
            case ("GET", "api/e2ee/secret_storage"): return storage
            case ("POST", "api/e2ee/secret_storage/slots"):
                reauthSeen.append(reauth)
                if failReauthOnce { failReauthOnce = false; throw E2eeAPIError(status: 403, code: "E2EE_REAUTH_REQUIRED") }
                var slot = body ?? [:]
                slot["created_at"] = .string("2026-11-26T00:00:00Z")
                slots.append(.object(slot))
            case ("DELETE", _) where path.hasPrefix("api/e2ee/secret_storage/slots/"):
                reauthSeen.append(reauth)
                let id = String(path.dropFirst("api/e2ee/secret_storage/slots/".count))
                slots.removeAll { $0.object?["id"]?.string == id }
            default: throw E2eeAPIError(status: 404, code: nil)
            }
            object["slots"] = .array(slots)
            storage = .object(object)
            return .null
        }
    }

    private func prepared() async throws -> (Server, UnlockedSecretStorage, String, E2eeKeySlots) {
        let fake = FakeSetupServer(userID: user)
        let device = try Device(fake)
        let start = try device.setup.begin()
        try await device.setup.completeSetup(start.session, reauth: "t")
        let server = Server(storage: try JSONDecoder().decode(JSONValue.self, from: Data(try await fake.secretStorage().utf8)))
        let parsed = try #require(try parseSecretStorage(userId: user, response: try await fake.secretStorage()))
        let unlocked = try parsed.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: start.recoveryKey))
        device.cleanUp()
        return (server, unlocked, start.recoveryKey, E2eeKeySlots(api: server, userID: user))
    }

    @Test func リカバリーキーを作り直すと新しいキーだけが解錠でき古いキーは使えなくなる() async throws {
        let (server, unlocked, firstKey, slots) = try await prepared()
        try await slots.load()
        #expect(slots.slots.count == 1 && slots.slots[0].kind == "recovery")

        let newKey = try await slots.regenerateRecovery(unlocked: unlocked, reauth: { "token" })
        // 追加と、古いスロットの削除のどちらも、再認証つき。リカバリーキーのスロットは 1 つのまま。
        #expect(server.reauthSeen == ["token", "token"])
        #expect(slots.slots.count == 1 && slots.slots[0].kind == "recovery")
        let storage = try #require(try parseSecretStorage(userId: user, response: String(decoding: try JSONEncoder().encode(server.storage), as: UTF8.self)))
        _ = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: newKey))
        #expect(throws: (any Error).self) { _ = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: firstKey)) }
        // 最後の 1 つは消せない。
        await #expect(throws: KeySlotError.lastSlot) { try await slots.remove(slots.slots[0], reauth: { "token3" }) }
    }

    @Test func 再認証が足りなければ取り直してもう一度だけ行う() async throws {
        let (server, unlocked, _, slots) = try await prepared()
        try await slots.load()
        server.failReauthOnce = true
        var tokens = 0
        _ = try await slots.regenerateRecovery(unlocked: unlocked, reauth: { tokens += 1; return "token\(tokens)" })
        // 追加が足りなければ取り直して 1 回だけ行い、そのあと、古いスロットを消す（もう 1 回）。
        #expect(server.reauthSeen == ["token1", "token2", "token3"] && slots.slots.count == 1)
    }
}
