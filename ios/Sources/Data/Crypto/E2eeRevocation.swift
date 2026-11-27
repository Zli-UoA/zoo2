import Foundation
import Observation
import ZooCrypto

/// 失効の範囲（docs/e2ee.md の失効の表）。
enum RevocationScopeChoice: String, Equatable, CaseIterable {
    /// 端末だけ（マスター鍵と SSSS 鍵は残す）。
    case deviceOnly = "device_only"
    /// パスキー、リカバリーキー、SSSS 鍵のどれかが漏れた（マスター鍵と SSSS 鍵も作り直す）。
    case full
}

/// 失効の画面に出す、進み具合と一覧（秘密鍵を含まない）。
struct RevocationSummary: Equatable {
    let scope: String
    let stage: String
    let deleted_devices: [String]
    let resigned_users: [String]
    let resigned_master_keys: [String]
    let versions_without_key: [String]
    let key_id: String
    let new_backup_version: String?
    let dropped_sessions: Int
    let revocation_id: String

    static let order = ["prepared", "secret_storage_saved", "cross_signing_saved", "signatures_saved", "backup_created", "migrated", "verified"]
    func reached(_ stage: String) -> Bool {
        (Self.order.firstIndex(of: self.stage) ?? 0) >= (Self.order.firstIndex(of: stage) ?? 0)
    }
}

extension RevocationSummary {
    init(_ value: ZooCrypto.RevocationSummary) {
        let stage: String
        switch value.stage {
        case .prepared: stage = "prepared"
        case .secretStorageSaved: stage = "secret_storage_saved"
        case .crossSigningSaved: stage = "cross_signing_saved"
        case .signaturesSaved: stage = "signatures_saved"
        case .backupCreated: stage = "backup_created"
        case .migrated: stage = "migrated"
        case .verified: stage = "verified"
        }
        self.init(scope: value.scope == .full ? "full" : "device_only", stage: stage,
                  deleted_devices: value.deletedDevices, resigned_users: value.resignedUsers,
                  resigned_master_keys: value.resignedMasterKeys, versions_without_key: value.versionsWithoutKey,
                  key_id: value.keyId, new_backup_version: value.newBackupVersion,
                  dropped_sessions: Int(clamping: value.droppedSessions), revocation_id: value.revocationId)
    }
}

private func rvSignature(_ signature: SignatureUpload) -> JSONValue {
    .object(["target_user_id": .string(signature.targetUserId), "target_key_id": .string(signature.targetKeyId),
             "signer_key_id": .string(signature.signerKeyId), "signature": .string(signature.signature)])
}

enum RevocationError: Error, Equatable {
    /// 別の端末の失効と入り混じった（409）。SSSS を開け直して、失効をやり直す。
    case conflict(String)
    /// 終わっていない失効がある（「失効を続ける」か、捨ててから始める）。
    case pending
    /// 続ける失効がない。
    case none
    /// 全範囲の失効に、鍵スロット（リカバリーキーかパスキー）がない。
    case noSlots
    /// 実行を始めた失効が、別の失効に置き換わった。
    case changed
}

/// 失効で使う API。`e2ee` の API と同じ要求を返す（封筒の `data` は外す）。
@MainActor
protocol E2eeRevocationAPI {
    func call(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?, reauth: String?,
              headers: [String: String]) async throws -> JSONValue
}

extension APIClient: E2eeRevocationAPI {
    func call(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?, reauth: String?,
              headers: [String: String]) async throws -> JSONValue {
        let data = try await e2ee(path, method: method, query: query, body: body, reauth: reauth, headers: headers)
        guard !data.isEmpty, let value = try? JSONDecoder().decode(JSONValue.self, from: data) else { return .null }
        if case .object(let object) = value, let inner = object["data"] { return inner }
        return value
    }
}

private func rvText(_ value: JSONValue) -> String { String(decoding: (try? JSONEncoder().encode(value)) ?? Data(), as: UTF8.self) }
private func rvValue(_ text: String) throws -> JSONValue { try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8)) }

/// 失効の手順（docs/e2ee.md の「失効の手順」。Web の `revocationController.ts` と同じ流れ）。
///
/// 秘密（マスター鍵の秘密鍵、SSSS 鍵、新しい鍵）は zoo-crypto の `Revocation` の中（Rust のメモリー）にだけ置く。
/// 保存が 1 つ済むたびに、進み具合を暗号ストアに保存し、アプリを閉じても SSSS を開け直して続けられる。
@Observable @MainActor
final class E2eeRevocation {
    private(set) var summary: RevocationSummary?

    @ObservationIgnored private let store: () -> CryptoStore?
    @ObservationIgnored private let api: any E2eeRevocationAPI
    @ObservationIgnored private let setup: E2eeSetup
    @ObservationIgnored private let userID: String
    @ObservationIgnored private var revocation: Revocation?
    @ObservationIgnored private weak var revocationStore: CryptoStore?
    @ObservationIgnored private var slots: [String] = []
    @ObservationIgnored private static let uploadBatch = 1000
    @ObservationIgnored private static let backupBatch = 1000

    init(store: @escaping () -> CryptoStore?, api: any E2eeRevocationAPI, setup: E2eeSetup, userID: String) {
        self.store = store
        self.api = api
        self.setup = setup
        self.userID = userID
    }

    private func currentStore() throws -> CryptoStore {
        guard let store = store(), store.isUsable else { throw E2eeStoreError.sendNeedsUnlock }
        return store
    }

    /// 保存した失効の進み具合（なければ `nil`）。
    func savedSummary() throws -> RevocationSummary? {
        try currentStore().revocationSummary().map(RevocationSummary.init)
    }

    // MARK: - 始める

    /// 始める前に SSSS を開け直す（パスキーかリカバリーキー）。
    func openStorage(_ method: UnlockMethod) async throws -> UnlockedSecretStorage {
        try await e2eeOpenSecretStorage(api: api, userID: userID, method: method)
    }

    /// 手順 1：選んだ端末を削除する（端末の削除は失効ではない）。
    func deleteDevices(_ ids: [String]) async throws {
        for id in ids {
            guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
            _ = try await api.call("api/e2ee/devices/\(id.lowercased())", method: "DELETE", query: [:], body: nil, reauth: nil, headers: [:])
        }
    }

    /// 失効を始める。署名し直す一覧を集めて暗号ストアに保存する（サーバーにはまだ何も保存しない）。
    func begin(scope: RevocationScopeChoice, deletedDevices: [String], unlocked: UnlockedSecretStorage) async throws -> RevocationSummary {
        let store = try currentStore()
        if try store.revocationSummary() != nil { throw RevocationError.pending }
        let keysQuery = try await ownKeysQuery()
        let versions = try await api.call("api/e2ee/key_backup/versions", method: "GET", query: [:], body: nil, reauth: nil, headers: [:])
        let pins = try await allPins()
        // 署名し直す候補：ピンのある相手と、この端末の信頼の記録がある相手（記録だけの相手のピンは、記録から作る）。
        var targets = Set(pins.compactMap { $0.object?["target_user_id"]?.string })
        targets.formUnion(store.recordedUsers())
        targets.remove(userID)
        let targetsQuery = targets.isEmpty ? JSONValue.object([:]) : try await queryKeys(Array(targets))
        let input = RevocationInput(scope: scope == .full ? .full : .deviceOnly, keysQuery: rvText(keysQuery),
                                    versions: rvText(versions), pins: pins.map(rvText),
                                    targetsQuery: rvText(targetsQuery), deletedDevices: deletedDevices)
        revocation = nil
        slots = []
        revocation = try store.beginRevocation(input: input, unlocked: unlocked)
        revocationStore = store
        return try refreshSummary()
    }

    /// 止まった失効を、開け直した SSSS で続ける。全範囲を SSSS の保存の前に止めていたら `RevocationRestart` を投げる。
    func resume(unlocked: UnlockedSecretStorage) throws -> RevocationSummary {
        let store = try currentStore()
        revocation = try store.resumeRevocation(unlocked: unlocked)
        revocationStore = store
        slots = []
        return try refreshSummary()
    }

    /// やり直すために、保存した失効を捨てる。
    func abandon() throws {
        revocation = nil
        slots = []
        try currentStore().clearRevocation()
        summary = nil
    }

    /// 全範囲：新しい SSSS 鍵のリカバリーキーを作る。利用者に控えてもらう。
    func newRecoveryKey() throws -> String {
        guard let revocation else { throw RevocationError.none }
        let recovery = RecoveryKey.generate()
        let slot = try newRecoveryKeySlot(userId: userID, ssssKey: revocation.ssssKey(), recoveryKey: recovery)
        slots = slots.filter { (try? rvValue($0))?.object?["kind"]?.string != "recovery" } + [slot]
        return recovery.toDisplay()
    }

    /// 全範囲：新しい SSSS 鍵に、この端末のパスキーのスロットを作る。
    func addPasskey(client: any PasskeyClient, rpID: String, label: String?) async throws {
        guard let revocation else { throw RevocationError.none }
        let salt = newPrfSalt()
        let result = try await client.prfForNewSlot(rpID: rpID, salt: salt)
        guard self.revocation === revocation else { throw RevocationError.changed }
        var slot = try rvValue(try newPasskeyKeySlot(userId: userID, ssssKey: revocation.ssssKey(), credentialId: result.credentialID,
                                                     prfSalt: salt, prfOutput: result.prfOutput))
        if case .object(var object) = slot { object["label"] = label.map(JSONValue.string); slot = .object(object) }
        slots = slots.filter { (try? rvValue($0))?.object?["credential_id"]?.string != result.credentialID } + [rvText(slot)]
    }

    // MARK: - 実行

    /// 記録した段の続きから、最後まで保存する。`reauth` は 5 分以内の再認証の ID トークン。
    /// 403 `E2EE_REAUTH_REQUIRED` は `E2eeSetupError.reauthRequired`、409 は `RevocationError.conflict`、
    /// 読み直しの不一致は `ZooCryptoError.ServerKeyMismatch`。
    @discardableResult
    func run(reauth: String?, progress: ((RevocationSummary) -> Void)? = nil) async throws -> RevocationSummary {
        guard let revocation else { throw RevocationError.none }
        let store = try currentStore()
        guard revocationStore === store else { throw RevocationError.none }
        var summary = try refreshSummary()
        let id = summary.revocation_id
        func advance() throws { summary = try refreshSummary(); progress?(summary) }
        var secretsRevision: UInt64?

        if !summary.reached("secret_storage_saved") {
            let body = try revocation.secretStorageRequest(slots: slots)
            let path = summary.scope == "full" ? "api/e2ee/secret_storage/keys" : "api/e2ee/secret_storage/secrets"
            do {
                let response = try await guarded { try await api.call(path, method: "POST", query: [:], body: rvObject(body), reauth: reauth, headers: [:]) }
                secretsRevision = response.object?["secrets_revision"]?.uint
            } catch RevocationError.conflict(let code) where code == "E2EE_SSSS_CONFLICT" {
                // 応答をなくして送り直したか、別の端末が先に書いた。読み直して、自分の書き込みが済んでいれば続ける。
                let current = try await api.call("api/e2ee/secret_storage", method: "GET", query: [:], body: nil, reauth: nil, headers: [:])
                guard try revocation.secretStorageMatches(secretStorage: rvText(current)) else { throw RevocationError.conflict(code) }
                secretsRevision = current.object?["secrets_revision"]?.uint
            }
            revocation.secretStorageSaved(secretsRevision: secretsRevision)
            try store.saveRevocation(revocation)
            try advance()
        }
        if secretsRevision == nil {
            secretsRevision = try await api.call("api/e2ee/secret_storage", method: "GET", query: [:], body: nil, reauth: nil, headers: [:])
                .object?["secrets_revision"]?.uint
        }

        if !summary.reached("cross_signing_saved") {
            let body = revocation.crossSigningRequest()
            _ = try await guarded { try await api.call("api/e2ee/cross_signing/keys", method: "POST", query: [:], body: rvObject(body), reauth: reauth, headers: [:]) }
            revocation.advance(stage: .crossSigningSaved)
            try store.applyRevocation(revocation, secretsRevision: secretsRevision)
            try advance()
        }

        if !summary.reached("signatures_saved") {
            // 今の端末への自己署名は、相手のマスター鍵への署名と分けて先に送る。
            var signatures = try revocation.signatures(deviceKeys: store.deviceKeysJSON())
            try await uploadSignatures([rvSignature(signatures.device)], reauth: reauth)
            // 失効を始めてから相手がマスター鍵を作り直していると、その署名はサーバーが 404 で拒む。送る前に今のマスター鍵と照合し、
            // 作り直されていたら外す。
            for attempt in 0..<3 {
                let targets = summary.resigned_master_keys
                if !targets.isEmpty {
                    let current = try await queryKeys(targets)
                    _ = try revocation.dropStaleMasterKeys(masterKeys: rvText(current.object?["master_keys"] ?? .object([:])))
                    try store.saveRevocation(revocation)
                    try advance()
                    signatures = try revocation.signatures(deviceKeys: store.deviceKeysJSON())
                }
                do {
                    try await uploadSignatures(signatures.masterKeys.map(rvSignature), reauth: reauth)
                    break
                } catch let failure as E2eeAPIError where failure.status == 404 && attempt < 2 {
                    continue
                }
            }
            let pins = try signatures.pins.map(rvValue)
            if !pins.isEmpty { try await uploadPins(pins, reauth: reauth) }
            revocation.advance(stage: .signaturesSaved)
            try store.saveRevocation(revocation)
            try advance()
        }

        if !summary.reached("backup_created") {
            let body = try revocation.backupVersionRequest()
            let created = try await guarded { try await api.call("api/e2ee/key_backup/versions", method: "POST", query: [:], body: rvObject(body), reauth: reauth, headers: [:]) }
            guard let version = created.object?["version"]?.string else { throw ZooError.invalidResponse }
            revocation.backupCreated(version: version)
            try store.saveRevocation(revocation)
            try advance()
        }

        if !summary.reached("migrated") {
            guard let newVersion = summary.new_backup_version else { throw ZooError.invalidResponse }
            let versions = try await api.call("api/e2ee/key_backup/versions", method: "GET", query: [:], body: nil, reauth: nil, headers: [:])
            var proof: String??
            for old in try revocation.versionsToMigrate(versions: rvText(versions)) {
                var dropped: UInt64 = 0
                let found = try await forEachBackupPage(old.version) { rows in
                    let encoded = rows.map(rvText)
                    let result = try await Task.detached(priority: .userInitiated) {
                        try revocation.reencryptRows(oldKey: old.key, rows: encoded)
                    }.value
                    try Task.checkCancellation()
                    dropped += result.dropped
                    if proof == nil {
                        let target = try await api.call("api/e2ee/key_backup/versions/\(newVersion)", method: "GET", query: [:], body: nil, reauth: nil, headers: [:])
                        if let object = target.object, object["upload_key"] != nil, object["upload_key"]?.string == nil { throw ZooError.invalidResponse }
                        if let uploadKey = target.object?["upload_key"]?.string {
                            proof = .some(try revocation.uploadProof(uploadKey: uploadKey, userId: userID, version: newVersion))
                        } else { proof = .some(nil) }
                    }
                    let migrated = try result.rows.map(rvValue)
                    var start = 0
                    while start < migrated.count {
                        let batch = Array(migrated[start..<min(start + Self.backupBatch, migrated.count)])
                        start += Self.backupBatch
                        var headers: [String: String] = [:]
                        if let value = proof ?? nil { headers["X-Zoo-Backup-Proof"] = value }
                        _ = try await guarded {
                            try await api.call("api/e2ee/key_backup/keys", method: "POST",
                                               query: ["version": newVersion, "secrets_revision": String(secretsRevision ?? 0)],
                                               body: ["sessions": .array(batch)], reauth: nil, headers: headers)
                        }
                    }
                    revocation.recordDropped(version: old.version, dropped: dropped)
                    try store.saveRevocation(revocation)
                }
                guard found else { continue }
                // 今の版ではないので、再認証は要らない。消えていれば（404）何もしない。
                do { _ = try await api.call("api/e2ee/key_backup/versions/\(old.version)", method: "DELETE", query: [:], body: nil, reauth: nil, headers: [:]) }
                catch let failure as E2eeAPIError where failure.status == 404 {}
            }
            revocation.advance(stage: .migrated)
            try store.saveRevocation(revocation)
            try advance()
        }

        // 読み直して、作った鍵と一致することを確かめる。一致したら記録を消す。
        let storage = try await api.call("api/e2ee/secret_storage", method: "GET", query: [:], body: nil, reauth: nil, headers: [:])
        let own = try await ownKeysQuery()
        let current = try? await api.call("api/e2ee/key_backup/versions/current", method: "GET", query: [:], body: nil, reauth: nil, headers: [:])
        try revocation.verify(secretStorage: rvText(storage), keysQuery: rvText(own), currentVersion: rvText(current ?? .object([:])))
        guard self.revocation === revocation, try store.revocationSummary()?.revocationId == id else { throw RevocationError.changed }
        try store.clearRevocation()
        self.revocation = nil
        slots = []
        self.summary = nil
        return RevocationSummary(scope: summary.scope, stage: "verified", deleted_devices: summary.deleted_devices,
                                 resigned_users: summary.resigned_users, resigned_master_keys: summary.resigned_master_keys,
                                 versions_without_key: summary.versions_without_key, key_id: summary.key_id,
                                 new_backup_version: summary.new_backup_version, dropped_sessions: summary.dropped_sessions,
                                 revocation_id: summary.revocation_id)
    }

    // MARK: - 内部

    /// 403 `E2EE_REAUTH_REQUIRED` と 409 を、手順の誤りに変える。
    private func guarded<T>(_ task: () async throws -> T) async throws -> T {
        do { return try await task() }
        catch let failure as E2eeAPIError {
            if failure.isReauthRequired { throw E2eeSetupError.reauthRequired }
            if failure.status == 409 { throw RevocationError.conflict(failure.code ?? "CONFLICT") }
            throw failure
        }
    }


    private func refreshSummary() throws -> RevocationSummary {
        guard let revocation else { throw RevocationError.none }
        let value = RevocationSummary(revocation.summary())
        summary = value
        return value
    }

    private func ownKeysQuery() async throws -> JSONValue {
        try await api.call("api/e2ee/keys/query", method: "POST", query: [:], body: ["user_ids": .array([.string(userID)])], reauth: nil, headers: [:])
    }

    /// 相手の `keys/query`（`master_keys` と `signatures` を使う）。
    private func queryKeys(_ users: [String]) async throws -> JSONValue {
        var master: [String: JSONValue] = [:]
        var signatures: [JSONValue] = []
        var start = 0
        while start < users.count {
            let chunk = Array(users[start..<min(start + 50, users.count)])
            start += 50
            let response = try await api.call("api/e2ee/keys/query", method: "POST", query: [:], body: ["user_ids": .array(chunk.map(JSONValue.string))], reauth: nil, headers: [:])
            guard response.object != nil else { throw ZooError.invalidResponse }
            if case .object(let keys)? = response.object?["master_keys"] { master.merge(keys) { _, new in new } }
            if case .array(let rows)? = response.object?["signatures"] { signatures += rows }
        }
        return .object(["master_keys": .object(master), "signatures": .array(signatures)])
    }

    private func allPins() async throws -> [JSONValue] {
        var pins: [JSONValue] = []
        var after: String?
        var seen = Set<String>()
        repeat {
            let page = try await api.call("api/e2ee/tofu_pins", method: "GET", query: after.map { ["after": $0] } ?? [:], body: nil, reauth: nil, headers: [:])
            pins += page.object?["pins"]?.array ?? []
            switch page.object?["next"] {
                case nil, .null: after = nil
                case .string(let value): after = value
                default: throw ZooError.invalidResponse
                }
            if let after, !seen.insert(after).inserted { throw ZooError.invalidResponse }
        } while after != nil
        return pins
    }

    private func forEachBackupPage(_ version: String, consume: ([JSONValue]) async throws -> Void) async throws -> Bool {
        var after: String?
        var seen = Set<String>()
        repeat {
            try Task.checkCancellation()
            var query = ["version": version]
            if let after { query["after"] = after }
            let page: JSONValue
            do { page = try await api.call("api/e2ee/key_backup/keys", method: "GET", query: query, body: nil, reauth: nil, headers: [:]) }
            catch let failure as E2eeAPIError where failure.status == 404 { return false }
            guard let sessions = page.object?["sessions"]?.array, sessions.count <= Self.backupBatch else { throw ZooError.invalidResponse }
            switch page.object?["next"] {
            case nil, .null: after = nil
            case .string(let value): after = value
            default: throw ZooError.invalidResponse
            }
            if let after, !seen.insert(after).inserted { throw ZooError.invalidResponse }
            try await consume(sessions)
        } while after != nil
        return true
    }

    private func uploadSignatures(_ signatures: [JSONValue], reauth: String?) async throws {
        var start = 0
        while start < signatures.count {
            let batch = Array(signatures[start..<min(start + Self.uploadBatch, signatures.count)])
            start += Self.uploadBatch
            _ = try await guarded { try await api.call("api/e2ee/cross_signing/signatures", method: "POST", query: [:], body: ["signatures": .array(batch)], reauth: reauth, headers: [:]) }
        }
    }

    /// 作り直したピン（`replace: true`）。サーバーは `type` と `user_id` を自分で付けるので、署名付きのピンから外して送る。
    private func uploadPins(_ pins: [JSONValue], reauth: String?) async throws {
        let body: [JSONValue] = pins.map { pin in
            let object = pin.object ?? [:]
            return .object(["target_user_id": object["target_user_id"] ?? .null, "master_key": object["master_key"] ?? .null, "signatures": object["signatures"] ?? .null])
        }
        var start = 0
        while start < body.count {
            let batch = Array(body[start..<min(start + Self.uploadBatch, body.count)])
            start += Self.uploadBatch
            _ = try await guarded { try await api.call("api/e2ee/tofu_pins", method: "POST", query: [:], body: ["pins": .array(batch), "replace": .bool(true)], reauth: reauth, headers: [:]) }
        }
    }
}

private func rvEncode(_ value: JSONValue) throws -> Data { try JSONEncoder().encode(value) }
private func rvObject(_ text: String) throws -> [String: JSONValue] {
    guard case .object(let object) = try rvValue(text) else { throw ZooError.invalidResponse }
    return object
}

/// パスキーかリカバリーキーで SSSS を開ける（保存はしない）。失効と、鍵スロットの追加・削除が使う。
@MainActor
func e2eeOpenSecretStorage(api: any E2eeRevocationAPI, userID: String, method: UnlockMethod) async throws -> UnlockedSecretStorage {
    let storage = String(decoding: try rvEncode(try await api.call("api/e2ee/secret_storage", method: "GET", query: [:], body: nil, reauth: nil, headers: [:])), as: UTF8.self)
    guard let parsed = try parseSecretStorage(userId: userID, response: storage) else { throw E2eeSetupError.noSecretStorage }
    switch method {
    case .recovery(let key):
        return try parsed.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: key))
    case .passkey(let client, let rpID):
        let slots = try parsed.passkeySlots()
        guard !slots.isEmpty else { throw PasskeyError.noSlot }
        let result = try await client.prfForUnlock(rpID: rpID, slots: slots)
        return try parsed.unlockWithPasskey(credentialId: result.credentialID, prfOutput: result.prfOutput)
    }
}
