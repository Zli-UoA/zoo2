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
        switch (method, path) {
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
    private func prepared() async throws -> (Device, RevocationServer, UnlockedSecretStorage, FakeSetupServer) {
        let fake = FakeSetupServer(userID: user)
        let device = try Device(fake)
        let start = try device.setup.begin()
        try await device.setup.completeSetup(start.session, reauth: "t")
        let server = try await RevocationServer(from: fake, userID: user)
        let storage = try #require(try parseSecretStorage(userId: user, response: try await fake.secretStorage()))
        let unlocked = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: start.recoveryKey))
        return (device, server, unlocked, fake)
    }

    @Test func 端末だけの失効は鍵を作り直して保存し記録を消す() async throws {
        let (device, server, unlocked, fake) = try await prepared()
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
        _ = fake
    }
}
