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
        return previewJSONText(.object(version))
    }

    func backupKeys(version: String, after: String?) async throws -> String {
        previewJSONText(.object(["sessions": .array(Array(backupRows.values)), "next": .null]))
    }

    func uploadBackupKeys(version: String, secretsRevision: UInt64, body: String, proof: String?) async throws {
        if let backupConflict { throw E2eeAPIError(status: 409, code: backupConflict) }
        guard case .object(let last)? = versions.last, last["version"] == .string(version), secretsRevision == revision else {
            throw E2eeAPIError(status: 409, code: "E2EE_WRONG_BACKUP_VERSION")
        }
        backupProofs.append(proof)
        guard case .array(let sessions)? = previewJSONObject(body)["sessions"] else { return }
        backupUploads.append(sessions.count)
        for session in sessions {
            guard case .object(let row) = session, case .string(let channel)? = row["channel_id"],
                  case .string(let sender)? = row["sender_key"], case .string(let id)? = row["session_id"] else { continue }
            backupRows["\(channel)|\(sender)|\(id)"] = session
        }
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

    func e2eeJSON(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?) async throws -> JSONValue {
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
    static func makePreview(userID: String, alreadySetUp: Bool = false, failingSetup: Bool = false) -> E2eeController {
        let server = PreviewE2eeServer(userID: userID)
        let controller = buildPreview(server: server, userID: userID)
        guard alreadySetUp else {
            controller.start()
            return controller
        }
        let other = buildPreview(server: server, userID: userID)
        Task { @MainActor in
            other.start()
            for _ in 0..<100 where other.status == .starting && !failingSetup { try? await Task.sleep(for: .milliseconds(100)) }
            do {
                // 準備ができて、初回設定が成功してから、対象の端末を始める。失敗は、テストから見えるエラーにする。
                guard other.status != .starting, !failingSetup else { throw E2eeSetupError.noSecretStorage }
                let start = try other.beginSetup()
                try await other.addSetupPasskey(start, label: "iPad（iOS）")
                try await other.completeSetup(start)
            } catch {
                // 失敗したときは、対象の端末を始めない（始めると、状態の取得が成功してエラーを消すため）。テストから見えるまま残す。
                controller.setPreviewFailure("テスト用の初期設定に失敗しました：\(error.localizedDescription)")
                await other.stop()
                return
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
        // 設定の「端末」「パスキーとリカバリーキー」「ユーザーの検証」「失効」の画面を、サーバーなしで動かす模擬。
        let calls = PreviewE2eeCalls(server: server)
        controller.devicesAPI = PreviewOwnDevices()
        controller.keySlots = E2eeKeySlots(api: calls, userID: userID)
        controller.revocation = E2eeRevocation(store: { manager.readyStore }, api: calls, setup: setup, userID: userID)
        controller.verification = E2eeVerification(store: { manager.readyStore }, api: PreviewVerificationAPI())
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

/// プレビュー用の、端末の一覧（2 台）。
@MainActor
struct PreviewOwnDevices: E2eeOwnDevicesAPI {
    func ownDevices() async throws -> [OwnDevice] {
        let json = #"[{"device_id":"11111111-1111-4111-8111-111111111111","display_name":"iPhone（iOS）","created_at":"2026-11-20T09:00:00Z","last_seen_at":"2026-11-26T08:30:00Z"},{"device_id":"22222222-2222-4222-8222-222222222222","display_name":"Chrome（macOS）","created_at":"2026-11-10T12:00:00Z","last_seen_at":"2026-11-25T21:10:00Z"},{"device_id":"33333333-3333-4333-8333-333333333333","display_name":"iPad（iOS）","created_at":"2026-10-30T07:00:00Z","last_seen_at":null}]"#
        return OwnDevice.ordered(try JSONDecoder().decode([OwnDevice].self, from: Data(json.utf8)))
    }
    func renameDevice(_ deviceID: String, to name: String) async throws {}
    func removeDevice(_ deviceID: String) async throws {}
}

/// プレビュー用の、鍵スロットと失効の API（読むものだけ。書き込みは何もしない）。
@MainActor
final class PreviewE2eeCalls: E2eeRevocationAPI {
    private let server: PreviewE2eeServer
    init(server: PreviewE2eeServer) { self.server = server }

    func call(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?, reauth: String?,
              headers: [String: String]) async throws -> JSONValue {
        if method == "GET", path == "api/e2ee/secret_storage" {
            var value = try JSONDecoder().decode(JSONValue.self, from: Data(try await server.secretStorage().utf8))
            // 表示のための、鍵スロットの追加の日時と、最後に使った日時。
            if case .object(var object) = value, case .array(let slots)? = object["slots"] {
                object["slots"] = .array(slots.map { slot in
                    guard case .object(var entry) = slot else { return slot }
                    entry["created_at"] = .string("2026-11-26T08:00:00Z")
                    entry["last_used_at"] = .string("2026-11-26T08:20:00Z")
                    return .object(entry)
                })
                value = .object(object)
            }
            return value
        }
        return .null
    }
}

/// プレビュー用の、検証の API（何も返さない。画面だけを動かす）。
@MainActor
struct PreviewVerificationAPI: E2eeVerificationAPI {
    func queryKeys(userID: String) async throws -> String { "{}" }
    func queryKeys(userIDs: [String]) async throws -> String { "{}" }
    func registerPin(_ pin: String) async throws -> Bool { true }
    func claim(_ oneTimeKeys: [String: JSONValue]) async throws -> [String: JSONValue] { [:] }
    func sendToDevice(deviceID: String, messages: [String: JSONValue]) async throws -> Int { 0 }
    func saveAccepted(pin: String, signature: SignatureUpload, reauth: String) async throws {}
}
