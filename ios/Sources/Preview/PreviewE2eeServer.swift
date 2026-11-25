import CryptoKit
import Foundation
import ZooCrypto

private func previewJSONText(_ value: JSONValue) -> String { String(decoding: try! JSONEncoder().encode(value), as: UTF8.self) }
private func previewJSONObject(_ json: String) -> [String: JSONValue] {
    guard case .object(let value) = try! JSONDecoder().decode(JSONValue.self, from: Data(json.utf8)) else { fatalError() }
    return value
}

/// プレビュー（`--uitesting-e2ee`）用の、状態を持つ模擬の E2EE サーバー。テストの `FakeSetupServer` と同じ作りで、
/// 初回設定と解錠の画面を、実際の zoo-crypto で動かす。
@MainActor
final class PreviewE2eeServer: E2eeSetupAPI, E2eeDeviceAPI, E2eeKeysAPI {
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

    private func fail(_ op: String) throws { if failNext.remove(op) != nil { throw ZooError.invalidResponse } }
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
        return previewJSONText(.object(response))
    }

    func createSecretStorageKey(_ body: String, reauth: String?) async throws -> UInt64? {
        try requireReauth(reauth)
        try fail("secret_storage")
        let body = previewJSONObject(body)
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
        let slot = previewJSONObject(slot)
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
        return previewJSONText(.object(response))
    }

    func backupVersions() async throws -> String { previewJSONText(.object(["versions": .array(versions)])) }

    func uploadCrossSigningKeys(_ body: String, reauth: String?) async throws {
        try requireReauth(reauth)
        let body = previewJSONObject(body)
        try checkDefaultKey(body)
        try fail("cross_signing")
        for field in ["master_key", "self_signing_key", "user_signing_key"] { if let value = body[field] { keys[field] = value } }
    }

    func uploadSignature(_ signature: String) async throws {
        try fail("signatures")
        let upload = previewJSONObject(signature)
        signatures.append(.object([
            "signer_user_id": .string(userID), "signer_key_id": upload["signer_key_id"]!,
            "target_user_id": upload["target_user_id"]!, "target_key_id": upload["target_key_id"]!,
            "signature": upload["signature"]!
        ]))
    }

    func createBackupVersion(_ body: String, reauth: String?) async throws {
        let body = previewJSONObject(body)
        if requireReauthForVersions { try requireReauth(reauth) }
        try checkDefaultKey(body)
        try fail("versions")
        versions.append(.object(["version": .string(String(versions.count + 1)), "algorithm": body["algorithm"]!,
                                 "auth_data": body["auth_data"]!, "deleted": .bool(false)]))
    }
    // MARK: 端末の登録と鍵（プレビューでは、受け取るだけ）

    private var registered = false
    func registerDevice(_ body: [String: JSONValue]) async throws { registered = true }
    func deviceIDs() async throws -> [String] { [] }
    func deleteDevice(_ deviceID: String) async throws {}
    func uploadKeys(deviceID: String, oneTimeKeys: [String: JSONValue], fallbackKeys: [String: JSONValue]) async throws -> KeyCounts {
        KeyCounts(oneTimeKeys: 50, hasUnusedFallbackKey: true)
    }
    func queryKeys(userIDs: [String]) async throws -> String { try await queryOwnKeys(userID: userID) }
    func ackToDevice(deviceID: String, ids: [String]) async throws {}
}

/// プレビュー用の、メモリーだけの保存鍵の置き場。
final class PreviewMemoryKey: StorageKeyBacking, @unchecked Sendable {
    private let lock = NSLock()
    private var value: Data?
    func load() throws -> Data? { lock.withLock { value } }
    func save(_ key: Data) throws { lock.withLock { value = key } }
    func delete() throws { lock.withLock { value = nil } }
}

/// プレビュー用の、配布先・鍵の払い出し・to-device・メッセージの API。自分の端末だけを配布先にして、受け取るだけ。
@MainActor
final class PreviewE2eeRequests: E2eeRequesting {
    let server: PreviewE2eeServer
    let device: () -> CryptoStore?
    init(server: PreviewE2eeServer, device: @escaping () -> CryptoStore?) { self.server = server; self.device = device }

    func e2eeJSON(_ path: String, method: String, body: [String: JSONValue]?) async throws -> JSONValue {
        if path.hasPrefix("api/e2ee/channels/") {
            guard case .object(var page) = try JSONDecoder().decode(JSONValue.self, from: Data(try await server.queryOwnKeys(userID: server.userID).utf8)),
                  let store = device() else { return .object([:]) }
            page["membership_epoch"] = .number(1)
            page["user_ids"] = .array([.string(server.userID)])
            page["device_keys"] = .object([server.userID: .object([
                store.deviceID: try JSONDecoder().decode(JSONValue.self, from: Data(store.deviceKeys().utf8))
            ])])
            return .object(page)
        }
        if path == "api/e2ee/to_device" { return .object(["undelivered": .array([])]) }
        return .object([:])
    }
}

extension E2eeController {
    /// `--uitesting-e2ee` 用。模擬サーバーで、実際の zoo-crypto を使って、初回設定と解錠の画面を動かす。
    /// `alreadySetUp` のときは、先に別の端末で設定を済ませてから、この端末を始める（解錠が要る状態になる）。
    static func makePreview(userID: String, alreadySetUp: Bool = false) -> E2eeController {
        let server = PreviewE2eeServer(userID: userID)
        let controller = buildPreview(server: server, userID: userID)
        guard alreadySetUp else {
            controller.start()
            return controller
        }
        let other = buildPreview(server: server, userID: userID)
        Task { @MainActor in
            other.start()
            for _ in 0..<100 where other.status == .starting { try? await Task.sleep(for: .milliseconds(100)) }
            if let start = try? other.beginSetup() {
                try? await other.addSetupPasskey(start, label: "iPad（iOS）")
                try? await other.completeSetup(start)
            }
            await other.stop()
            controller.start()
        }
        return controller
    }

    private static func buildPreview(server: PreviewE2eeServer, userID: String) -> E2eeController {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("zoo-preview-e2ee-\(UUID().uuidString)", isDirectory: true)
        let files = CryptoStoreFiles(directory: directory, keys: StorageKeyStore(backing: PreviewMemoryKey()))
        let manager = E2eeDeviceManager(api: server, files: files, userID: userID,
                                        observeToDevice: { _, receive in
                                            receive([])
                                            try await Task.sleep(for: .seconds(3600))
                                        })
        let setup = E2eeSetup(api: server, userID: userID, store: { try await manager.waitForStore() })
        let encryption = MessageEncryption(store: { manager.readyStore }, api: PreviewE2eeRequests(server: server, device: { manager.readyStore }))
        let controller = E2eeController(manager: manager, setup: setup, encryption: encryption)
        // 再認証は、ブラウザーを開かずに通す。
        controller.reauthenticate = { "preview-token" }
        controller.passkeys = PreviewPasskeyClient()
        controller.rpID = "account.zli.works"
        return controller
    }
}

/// プレビュー用の模擬パスキー。システムのパスキーの画面を出さず、スロットごとの塩から決まる PRF の出力を返す
/// （同じ塩なら同じ出力なので、足したスロットで解錠できる）。シミュレーターは、パスキーの保存先を持たない。
@MainActor
final class PreviewPasskeyClient: PasskeyClient {
    private func output(_ salt: Data) -> Data { Data(SHA256.hash(data: Data("zoo-preview-prf".utf8) + salt)) }

    func prfForNewSlot(rpID: String, salt: Data) async throws -> PasskeyResult {
        try await Task.sleep(for: .milliseconds(400))
        return PasskeyResult(credentialID: PasskeyCoding.base64URL(Data(SHA256.hash(data: salt)).prefix(16)), prfOutput: output(salt))
    }

    func prfForUnlock(rpID: String, slots: [PasskeySlot]) async throws -> PasskeyResult {
        try await Task.sleep(for: .milliseconds(400))
        guard let slot = slots.first else { throw PasskeyError.noSlot }
        return PasskeyResult(credentialID: slot.credentialId, prfOutput: output(slot.prfSalt))
    }
}
