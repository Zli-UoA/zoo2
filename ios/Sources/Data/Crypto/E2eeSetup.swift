import Foundation
import ZooCrypto

/// 初回設定と解錠で使う E2EE の API（docs/e2ee.md の「クロス署名」「Secret Storage」「鍵バックアップ」。#360）。
/// Android の `E2eeSetupApi`、React の `secretStorageApi.ts` と同じ。読み取りは本文（JSON）をそのまま返し、
/// 書き込みは `{"data": ...}` の封筒に入っている。直前の再認証が要る要求には、`reauth` に ID トークンを渡す（`X-Zoo-Reauth`）。
@MainActor
protocol E2eeSetupAPI {
    /// `GET /api/e2ee/secret_storage`。SSSS がまだなければ `default_key_id` が `null`。
    func secretStorage() async throws -> String
    /// `POST /api/e2ee/secret_storage/keys`。増やしたあとの `secrets_revision`。
    func createSecretStorageKey(_ body: String, reauth: String?) async throws -> UInt64?
    /// `POST /api/e2ee/secret_storage/slots`。
    func addKeySlot(_ slot: String, reauth: String?) async throws
    /// `POST /api/e2ee/secret_storage/slots/{slot_id}/used`。表示用の記録なので、失敗は呼び出し側が無視する。
    func markKeySlotUsed(_ slotID: String) async throws
    /// 自分だけを問い合わせた `POST /api/e2ee/keys/query`。
    func queryOwnKeys(userID: String) async throws -> String
    /// `GET /api/e2ee/key_backup/versions`（消した版を含むすべての版）。
    func backupVersions() async throws -> String
    /// `POST /api/e2ee/cross_signing/keys`。
    func uploadCrossSigningKeys(_ body: String, reauth: String?) async throws
    /// `POST /api/e2ee/cross_signing/signatures`（1 件）。
    func uploadSignature(_ signature: String) async throws
    /// `POST /api/e2ee/key_backup/versions`。
    func createBackupVersion(_ body: String, reauth: String?) async throws
}

/// 初回設定と解錠の誤り。
enum E2eeSetupError: Error, Equatable {
    /// 直前の再認証が要る。同じ鍵のまま、再認証してやり直す。
    case reauthRequired
    /// 別の端末が先に初回設定を始めた。手元の鍵を捨てる。表示したリカバリーキーは使えない。
    case conflict
    /// サーバーの鍵が SSSS の鍵と違う（失効が途中か、SSSS が書き換えられた）。
    case serverKeyMismatch(String)
    /// SSSS がまだない（解錠するものがない）。
    case noSecretStorage
}

/// 今の状態（React の `E2eeStatus`、Android の `E2eeStatus` と同じ）。
enum E2eeStatus: Equatable {
    /// SSSS がまだない。初回設定を始める。
    case notSetUp
    /// SSSS はあるが、この端末は解錠していない。
    case locked(LockReason)
    /// 解錠済み。
    case unlocked
}

enum LockReason: Equatable {
    /// まだ解錠していない（新しい端末）。
    case new
    /// 解錠済みの端末が、初回設定の途中で止まったのを見つけた。SSSS を開け直して続ける。
    case resume
    /// 暗号ストアの鍵とサーバーの鍵が違う（失効のあとか、改ざん）。
    case mismatch
}

/// [E2eeSetup.unlock] の結果。
struct UnlockResult {
    /// 開けた SSSS（続きの保存に使う）。
    let session: SetupSession
    let secretsRevision: UInt64?
    /// 続きの保存（初回設定の再開でクロス署名鍵がない）に再認証が要る。`false` なら最後まで保存してある。
    let needsReauth: Bool
}

/// 解錠の手段。
enum UnlockMethod {
    case recovery(String)
    /// パスキー。`rpID` は AuthZ のホスト名。
    case passkey(any PasskeyClient, rpID: String)
}

/// パスキーの PRF（`ios/Sources/Data/Crypto/Passkeys.swift`）。
@MainActor
protocol PasskeyClient {
    /// 新しいパスキーを作り、PRF の出力を返す。
    func prfForNewSlot(rpID: String, salt: Data) async throws -> PasskeyResult
    /// 保存したスロットのパスキーで PRF の出力を返す。
    func prfForUnlock(rpID: String, slots: [PasskeySlot]) async throws -> PasskeyResult
}

struct PasskeyResult {
    let credentialID: String
    let prfOutput: Data
}

enum PasskeyError: Error, Equatable {
    /// 使えるパスキーのスロットがない。
    case noSlot
    /// この OS では PRF を使えない（iOS 18 未満など）。
    case unsupported
    /// 利用者が取りやめた。
    case cancelled
    /// PRF の出力を返さない保存先だった。
    case noPRF
}

private func jsonObject(_ text: String) throws -> [String: JSONValue] {
    guard case .object(let object) = try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8)) else {
        throw ZooError.invalidResponse
    }
    return object
}

private func jsonText(_ value: JSONValue) throws -> String {
    String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
}

private extension [String: JSONValue] {
    func string(_ key: String) -> String? {
        if case .string(let value) = self[key] { return value }
        return nil
    }
}

/// 初回設定と解錠の手順（docs/e2ee.md の「初回設定と解錠の流れ」。#360）。Web の `setupController.ts`、TUI の
/// `e2ee/setup.rs`、Android の `E2eeSetup` と同じ流れで、保存の順と冪等の扱いを合わせる。
///
/// 秘密（マスター鍵の秘密鍵、SSSS 鍵）は UniFFI の `SetupSession`（Rust のメモリー）にだけ置き、ここでは
/// それが作った要求の本文をサーバーに送る。鍵バックアップの送信と取り込みは、送受信の実装（#361）で足す。
@MainActor
final class E2eeSetup {
    let api: any E2eeSetupAPI
    let userID: String
    private let store: () async throws -> CryptoStore

    init(api: any E2eeSetupAPI, userID: String, store: @escaping () async throws -> CryptoStore) {
        self.api = api
        self.userID = userID
        self.store = store
    }

    /// SSSS があるか（`default_key_id` が空でない）。
    static func hasSecretStorage(_ storage: String) -> Bool {
        guard let object = try? jsonObject(storage), let id = object.string("default_key_id") else { return false }
        return !id.isEmpty
    }

    private static func secretsRevision(_ storage: String) -> UInt64? {
        guard let object = try? jsonObject(storage), case .number(let value) = object["secrets_revision"] else { return nil }
        return UInt64(value)
    }

    private func serverState() async throws -> (keys: String, versions: String) {
        let keys = try await api.queryOwnKeys(userID: userID)
        return (keys, try await api.backupVersions())
    }

    /// 今の状態を読む。解錠済みの端末は、起動時の確認（署名し直し、止まった初回設定の発見）も行う。
    func loadStatus() async throws -> E2eeStatus {
        let store = try await store()
        // 取得を始める前の記録を使う（取得を待つ間に解錠し直して保存した新しい値を、古い応答で消さないため）。
        let recorded = store.secretsRevision
        return try await startupStatus(store, recorded: recorded, storage: api.secretStorage())
    }

    /// `GET /api/e2ee/secret_storage` の応答と暗号ストアから状態を決め、要るなら自分の端末に署名し直す。
    /// 解錠済みの端末でも、最後に解錠したときの `secrets_revision` がサーバーの値と違えば、暗号ストアの鍵を
    /// 使わずに解錠し直しを求める（古い値のまま鍵バックアップに送り続けないため）。
    private func startupStatus(_ store: CryptoStore, recorded: UInt64?, storage: String) async throws -> E2eeStatus {
        guard Self.hasSecretStorage(storage) else { return .notSetUp }
        guard store.hasOwnKeys else { return .locked(.new) }
        func mismatch() throws -> E2eeStatus {
            if let recorded { try store.forgetSecretsRevision(recorded) }
            return .locked(.mismatch)
        }
        if Self.secretsRevision(storage) != recorded { return try mismatch() }
        let state = try await serverState()
        switch try store.startupCheck(keysQuery: state.keys, versions: state.versions) {
        case nil: return .locked(.new)
        case .ready: return .unlocked
        case .signDevice(let signature):
            try await api.uploadSignature(signature)
            return .unlocked
        case .resumeSetup: return .locked(.resume)
        case .unlock: return try mismatch()
        }
    }

    /// 初回設定を始める。返したリカバリーキーを利用者に控えてもらう。
    func begin() throws -> SetupStart { try beginSetup(userId: userID) }

    /// 初回設定に、パスキー（PRF）のスロットを足す。
    func addSetupPasskey(_ session: SetupSession, client: any PasskeyClient, rpID: String, label: String?) async throws {
        let salt = newPrfSalt()
        let result = try await client.prfForNewSlot(rpID: rpID, salt: salt)
        try session.addPasskeySlot(credentialId: result.credentialID, prfSalt: salt, prfOutput: result.prfOutput, label: label)
    }

    /// 初回設定を保存する。SSSS、クロス署名鍵、端末への署名、バックアップの版の順。
    ///
    /// - 403 `E2EE_REAUTH_REQUIRED` なら `E2eeSetupError.reauthRequired`。同じ鍵のまま再認証してやり直す。
    /// - SSSS の保存が 409 `E2EE_SSSS_CONFLICT` なら、読み直して、自分の鍵が保存済みなら続ける。別の鍵なら
    ///   `E2eeSetupError.conflict`（手元の鍵を捨てる。表示したリカバリーキーは使えない）。
    func completeSetup(_ session: SetupSession, reauth: String?) async throws {
        let body = try session.secretStorageRequest()
        let revision: UInt64?
        do {
            revision = try await api.createSecretStorageKey(body, reauth: reauth)
        } catch let failure as E2eeAPIError {
            guard failure.isSsssConflict else { throw Self.mapReauth(failure) }
            // 応答をなくして送り直したか、別の端末が先に保存した。読み直して判定する。
            let current = try await api.secretStorage()
            let object = try jsonObject(current)
            guard object.string("default_key_id") == session.keyId() else { throw E2eeSetupError.conflict }
            // 前の試行で SSSS を保存したあとに足したスロットは、まだサーバーにない（同じ ID の再送は何もしない）。
            var saved = Set<String>()
            if case .array(let slots) = object["slots"] {
                for case .object(let slot) in slots { if let id = slot.string("id") { saved.insert(id) } }
            }
            for pending in session.pendingSlots() {
                guard let id = try jsonObject(pending).string("id"), !saved.contains(id) else { continue }
                do { try await api.addKeySlot(pending, reauth: reauth) } catch let failure as E2eeAPIError {
                    throw Self.mapReauth(failure)
                }
            }
            revision = Self.secretsRevision(current)
        }
        try await saveRemaining(session, secretsRevision: revision, reauth: reauth)
    }

    /// SSSS を保存したあと（または解錠したあと）の続きを保存し、自分の鍵を暗号ストアに保存する。応答をなくして
    /// 送り直しても、同じ鍵なら成功する（サーバーの冪等の規則）。続きの保存は、作った（開けた）SSSS 鍵が
    /// サーバーの既定の鍵のままであることを前提にする（`expected_default_key_id`）。
    func saveRemaining(_ session: SetupSession, secretsRevision: UInt64?, reauth: String?) async throws {
        let plan = try await planRemaining(session)
        let keyID = session.keyId()
        func withPrecondition(_ body: String) throws -> String {
            var object = try jsonObject(body)
            object["expected_default_key_id"] = .string(keyID)
            return try jsonText(.object(object))
        }
        do {
            if let body = plan.crossSigning { try await api.uploadCrossSigningKeys(withPrecondition(body), reauth: reauth) }
            if let signature = plan.deviceSignature { try await api.uploadSignature(signature) }
            if let body = plan.backupVersion { try await api.createBackupVersion(withPrecondition(body), reauth: reauth) }
        } catch let failure as E2eeAPIError {
            if failure.isSsssConflict { throw E2eeSetupError.conflict }
            throw Self.mapReauth(failure)
        }
        try await store().saveOwnKeys(session, secretsRevision: secretsRevision)
    }

    private func planRemaining(_ session: SetupSession) async throws -> ResumePlan {
        let server = try await serverState()
        let current = try await api.secretStorage()
        // 別の端末が SSSS を作り直した（同時の初回設定など）。手元の鍵でクロス署名鍵や版を差し替えると、
        // SSSS とサーバーの鍵の世代がずれるので、何も送らない（読んでから送るまでの間の競合は、
        // `expected_default_key_id` でサーバーが防ぐ）。
        guard try jsonObject(current).string("default_key_id") == session.keyId() else { throw E2eeSetupError.conflict }
        let deviceKeys = try await store().deviceKeys()
        do {
            return try session.plan(keysQuery: server.keys, versions: server.versions, deviceKeys: deviceKeys)
        } catch ZooCryptoError.ServerKeyMismatch(let reason) {
            throw E2eeSetupError.serverKeyMismatch(reason)
        }
    }

    /// パスキーかリカバリーキーで SSSS を開ける。続きの保存に再認証が要らなければ、そのまま最後まで保存する。
    /// 要るとき（初回設定の再開でクロス署名鍵がない）は `needsReauth` を真にして返すので、利用者の操作で
    /// 再認証してから `saveRemaining` を呼ぶ。
    func unlock(_ method: UnlockMethod) async throws -> UnlockResult {
        let storage = try await api.secretStorage()
        guard Self.hasSecretStorage(storage) else { throw E2eeSetupError.noSecretStorage }
        let session: SetupSession
        let slotID: String?
        switch method {
        case .recovery(let key):
            guard let opened = try unlockSetupWithRecoveryKey(userId: userID, response: storage, recoveryKey: key) else {
                throw E2eeSetupError.noSecretStorage
            }
            session = opened
            slotID = Self.slotID(storage, kind: "recovery")
        case .passkey(let client, let rpID):
            (session, slotID) = try await unlockWithPasskey(storage, client: client, rpID: rpID)
        }
        if let slotID { try? await api.markKeySlotUsed(slotID) }
        let revision = session.secretsRevision()
        let plan = try await planRemaining(session)
        if plan.crossSigning != nil { return UnlockResult(session: session, secretsRevision: revision, needsReauth: true) }
        try await saveRemaining(session, secretsRevision: revision, reauth: nil)
        return UnlockResult(session: session, secretsRevision: revision, needsReauth: false)
    }

    private static func slotID(_ storage: String, kind: String) -> String? {
        guard let object = try? jsonObject(storage), case .array(let slots) = object["slots"] else { return nil }
        for case .object(let slot) in slots where slot.string("kind") == kind { return slot.string("id") }
        return nil
    }

    private func unlockWithPasskey(_ storage: String, client: any PasskeyClient, rpID: String) async throws -> (SetupSession, String?) {
        guard let parsed = try parseSecretStorage(userId: userID, response: storage) else { throw E2eeSetupError.noSecretStorage }
        let slots = try parsed.passkeySlots()
        guard !slots.isEmpty else { throw PasskeyError.noSlot }
        let result = try await client.prfForUnlock(rpID: rpID, slots: slots)
        guard let slot = slots.first(where: { $0.credentialId == result.credentialID }) else { throw PasskeyError.noSlot }
        let unlocked = try parsed.unlockWithPasskey(credentialId: result.credentialID, prfOutput: result.prfOutput)
        return (try SetupSession.fromUnlocked(unlocked: unlocked), slot.slotId)
    }

    private static func mapReauth(_ failure: E2eeAPIError) -> Error {
        failure.isReauthRequired ? E2eeSetupError.reauthRequired : failure
    }
}

extension APIClient: E2eeSetupAPI {
    func secretStorage() async throws -> String { String(decoding: try await e2ee("api/e2ee/secret_storage", method: "GET"), as: UTF8.self) }

    func createSecretStorageKey(_ body: String, reauth: String?) async throws -> UInt64? {
        let data = try await e2ee("api/e2ee/secret_storage/keys", method: "POST", body: jsonObject(body), reauth: reauth)
        // 書き込みの応答は `{"data": {...}}` の封筒に入っている。
        guard case .object(let root) = try? JSONDecoder().decode(JSONValue.self, from: data) else { return nil }
        let inner: [String: JSONValue]
        if case .object(let object) = root["data"] { inner = object } else { inner = root }
        if case .number(let value) = inner["secrets_revision"] { return UInt64(value) }
        return nil
    }

    func addKeySlot(_ slot: String, reauth: String?) async throws {
        var body = try jsonObject(slot)
        // サーバーは `label: null` を受け付けないので、ないときは送らない。
        if body["label"] == .null { body["label"] = nil }
        _ = try await e2ee("api/e2ee/secret_storage/slots", method: "POST", body: body, reauth: reauth)
    }

    func markKeySlotUsed(_ slotID: String) async throws {
        guard let encoded = slotID.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed.subtracting(CharacterSet(charactersIn: "/"))) else {
            throw ZooError.invalidResponse
        }
        _ = try await e2ee("api/e2ee/secret_storage/slots/\(encoded)/used", method: "POST", body: [:])
    }

    func queryOwnKeys(userID: String) async throws -> String {
        String(decoding: try await e2ee("api/e2ee/keys/query", method: "POST", body: ["user_ids": .array([.string(userID)])]), as: UTF8.self)
    }

    func backupVersions() async throws -> String {
        String(decoding: try await e2ee("api/e2ee/key_backup/versions", method: "GET"), as: UTF8.self)
    }

    func uploadCrossSigningKeys(_ body: String, reauth: String?) async throws {
        _ = try await e2ee("api/e2ee/cross_signing/keys", method: "POST", body: jsonObject(body), reauth: reauth)
    }

    func uploadSignature(_ signature: String) async throws {
        let value = try JSONDecoder().decode(JSONValue.self, from: Data(signature.utf8))
        _ = try await e2ee("api/e2ee/cross_signing/signatures", method: "POST", body: ["signatures": .array([value])])
    }

    func createBackupVersion(_ body: String, reauth: String?) async throws {
        _ = try await e2ee("api/e2ee/key_backup/versions", method: "POST", body: jsonObject(body), reauth: reauth)
    }
}
