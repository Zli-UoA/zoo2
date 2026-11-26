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
    /// `GET /api/e2ee/key_backup/versions/current`。今の版がなければ（404）`nil`。
    func currentBackupVersion() async throws -> String?
    /// `GET /api/e2ee/key_backup/keys?version=<版>&after=<カーソル>`。`{"sessions": [...], "next": <カーソル>|null}`。
    func backupKeys(version: String, after: String?) async throws -> String
    /// `POST /api/e2ee/key_backup/keys?version=<版>&secrets_revision=<数>`。`proof` は `X-Zoo-Backup-Proof`（あれば）。
    func uploadBackupKeys(version: String, secretsRevision: UInt64, body: String, proof: String?) async throws
}

/// [E2eeSetup.uploadKeyBackup] の結果。
enum BackupUploadResult: Equatable, Sendable {
    case done(Int)
    /// 今の版を使えない（署名がない、SSSS の鍵と違う、巻き戻りなど）。送っていない。
    case rejected(String)
    /// サーバーが拒んだ（409 `E2EE_WRONG_BACKUP_VERSION` か `E2EE_SSSS_CONFLICT`）。
    case conflict(String)
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
    /// SSSS が書き換えられたと分かったが、状態を読み直せなかった（解錠済みとは言えない）。
    case secretsChanged
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
    /// 保留中の要求を取り消す（シートを閉じたとき）。
    func cancel()
    /// 新しいパスキーを作り、PRF の出力を返す。
    func prfForNewSlot(rpID: String, salt: Data) async throws -> PasskeyResult
    /// 保存したスロットのパスキーで PRF の出力を返す。
    func prfForUnlock(rpID: String, slots: [PasskeySlot]) async throws -> PasskeyResult
}

extension PasskeyClient {
    func cancel() {}
}

struct PasskeyResult {
    let credentialID: String
    let prfOutput: Data
}

enum PasskeyError: Error, Equatable {
    /// 利用者が取りやめた、または使えるパスキーがなかった。
    case cancelled
    /// この OS か認証器が PRF に対応していない（iOS 18 未満、PRF の結果がない）。
    case prfUnsupported
    /// このアプリから RP ID のパスキーを使えない（Associated Domains の関連付けがない）。
    case originNotAllowed
    /// 返ったパスキーに対応するスロットがない。
    case noSlot
    case unknown
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
        return UInt64(exactly: value)
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
        // 待っているあいだに、解錠し直して新しい `secrets_revision` を保存していたら、古い確認結果は使わず読み直す。
        if store.secretsRevision != recorded { return try await loadStatus() }
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

    // MARK: - 鍵バックアップ

    /// 1 回に送るセッションの数（サーバーの上限）。
    static let backupUploadBatch = 1000
    /// 鍵バックアップの取り込みの上限（docs/e2ee.md）。
    static let backupImportMaxPages = 1000
    static let backupImportMaxRows = 1000
    /// 鍵バックアップの送信と取り込みを 1 つずつにする（呼び出し元が端末の管理でも画面でも、重ねない）。
    private var backupChain: Task<Void, Never>?

    private func exclusive<T: Sendable>(_ operation: @escaping @MainActor () async throws -> T) async throws -> T {
        let previous = backupChain
        let task = Task { @MainActor in
            await previous?.value
            return try await operation()
        }
        backupChain = Task { _ = try? await task.value }
        return try await task.value
    }

    /// 起動時の確認（docs/e2ee.md の「初回設定と解錠の流れ」）。解錠済みの端末だけが、自分の端末への署名を確かめ、
    /// なければ署名し直し、送信待ちのセッションを鍵バックアップに送り、途中で止まっていた取り込みを続ける。
    /// まだ解錠していない端末は、何も取得しない（`nil`）。
    func runStartupCheck() async throws -> E2eeStatus? {
        let store = try await store()
        guard store.hasOwnKeys else { return nil }
        let recorded = store.secretsRevision
        let status = try await startupStatus(store, recorded: recorded, storage: api.secretStorage())
        guard status == .unlocked else { return status }
        // 鍵バックアップの失敗は、確かめ終えた解錠の状態を失わせない（通信に失敗しても、暗号化チャンネルは使える。次の機会に再び試す）。
        // ただし、SSSS が書き換えられていたら（`E2EE_SSSS_CONFLICT`）、解錠済みとは言えないので、状態を読み直す。
        var ssssConflict = false
        do {
            if case .conflict(let code) = try await uploadKeyBackup(), code == "E2EE_SSSS_CONFLICT" { ssssConflict = true }
            else { _ = try await importKeyBackup() }
        } catch is CancellationError {
            throw CancellationError()
        } catch let failure as E2eeAPIError where failure.code == "E2EE_SSSS_CONFLICT" {
            ssssConflict = true
        } catch {}
        // 読み直しの失敗は、解錠済みとして返さず、そのまま投げる（古い `.unlocked` を残さない）。
        if ssssConflict {
            do { return try await loadStatus() }
            catch is CancellationError { throw CancellationError() }
            catch { throw E2eeSetupError.secretsChanged }
        }
        return status
    }

    /// 鍵バックアップの今の版から、過去のセッションを取り込む（「新しい端末での解錠」の手順 5）。今の版は、送信と同じく
    /// `BackupUploadState.accept_current_version` で検証してから使う。この版から取り込み終えていれば何もしない。
    /// 返すのは取り込めた行の数。
    func importKeyBackup() async throws -> Int {
        try await exclusive {
            let store = try await self.store()
            guard store.hasOwnKeys else { return 0 }
            let current = try await self.api.currentBackupVersion()
            let version: String?
            do { version = try store.backupVersionToImport(currentVersion: current) } catch is ZooCryptoError {
                // 署名がない、鍵が違うなど、使えない版からは取り込まない。
                return 0
            }
            guard let version else { return 0 }
            var imported = 0
            var after: String?
            var seen = Set<String>()
            var pages = 0
            repeat {
                // 終わらないカーソルで、排他を占有し続けない（総ページ数は 1000、1 ページの行数は 1000 まで）。
                pages += 1
                guard pages <= Self.backupImportMaxPages else { throw ZooError.invalidResponse }
                let page = try await self.api.backupKeys(version: version, after: after)
                guard case .object(let object) = try JSONDecoder().decode(JSONValue.self, from: Data(page.utf8)),
                      case .array(let sessions)? = object["sessions"] else { throw ZooError.invalidResponse }
                // `next` は文字列か null だけ。ほかの値は不正なページなので、取り込み完了を記録しない。
                switch object["next"] {
                case nil, .null?: after = nil
                case .string(let next)?: after = next
                default: throw ZooError.invalidResponse
                }
                guard sessions.count <= Self.backupImportMaxRows else { throw ZooError.invalidResponse }
                imported += try store.importBackupRows(try sessions.map(jsonText))
                // 同じカーソルを返し続けるサーバーに、閉じ込められない（排他を占有し続けない）。
                if let after, !seen.insert(after).inserted { throw ZooError.invalidResponse }
            } while after != nil
            try store.backupImported(version)
            return imported
        }
    }

    /// 鍵バックアップの送信待ちのセッションを、今の版に送る（docs/e2ee.md の「鍵バックアップ」）。解錠していない端末と、
    /// 今の版がない場合は何もしない。送信は 1 つずつ。今の版に `upload_key` があれば、証明を `X-Zoo-Backup-Proof` に入れる。
    func uploadKeyBackup() async throws -> BackupUploadResult {
        try await exclusive {
            let store = try await self.store()
            guard store.hasOwnKeys else { return .done(0) }
            let current = try await self.api.currentBackupVersion()
            var uploaded = 0
            while true {
                switch try store.prepareBackupUpload(currentVersion: current, limit: Self.backupUploadBatch) {
                case .none: return .done(uploaded)
                case .rejected(let reason): return .rejected(reason)
                case .upload(let id, let version, let revision, let body, let count, let proof):
                    do {
                        try await self.api.uploadBackupKeys(version: version, secretsRevision: revision, body: body, proof: proof)
                    } catch let failure as E2eeAPIError {
                        let conflict = failure.status == 409 && (failure.code == "E2EE_WRONG_BACKUP_VERSION" || failure.code == "E2EE_SSSS_CONFLICT")
                        store.backupUploadFailed(id, conflict: conflict)
                        // SSSS の秘密が書き換えられた（失効など）。解錠し直すまで送らない。
                        if failure.code == "E2EE_SSSS_CONFLICT" { try store.forgetSecretsRevision(revision) }
                        if conflict, let code = failure.code { return .conflict(code) }
                        throw failure
                    } catch {
                        store.backupUploadFailed(id, conflict: false)
                        throw error
                    }
                    try store.backupUploaded(id)
                    uploaded += count
                }
            }
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


    // MARK: - E2EE のリセット（docs/e2ee.md の「失効の手順」）

    /// リセットの前提にする、サーバーの今の SSSS（`GET /api/e2ee/secret_storage` で読んだ既定の鍵と `secrets_revision`）。
    struct ResetExpectation: Equatable {
        let defaultKeyID: String?
        let secretsRevision: UInt64?
    }

    /// E2EE のリセットを始める。新しい鍵とリカバリーキーを作る（利用者にリカバリーキーを控えてもらう）。
    func beginReset() async throws -> (start: SetupStart, expectation: ResetExpectation) {
        let current = try await api.secretStorage()
        let object = try jsonObject(current)
        let expectation = Self.hasSecretStorage(current)
            ? ResetExpectation(defaultKeyID: object.string("default_key_id"), secretsRevision: Self.secretsRevision(current))
            : ResetExpectation(defaultKeyID: nil, secretsRevision: nil)
        return (try begin(), expectation)
    }

    /// E2EE のリセットを保存する。SSSS の作り直し（サーバーの今の鍵と版が前提）、クロス署名鍵 3 つの差し替え、今の端末への署名、
    /// 新しい鍵バックアップの版の順。403 `E2EE_REAUTH_REQUIRED` は `E2eeSetupError.reauthRequired`（同じ鍵のまま再認証してやり直す）、
    /// SSSS の 409 は、読み直して自分の鍵が保存済みなら続け、別の鍵なら `E2eeSetupError.conflict`。
    func completeReset(_ session: SetupSession, expectation: ResetExpectation, reauth: String?) async throws {
        let body = try session.resetSecretStorageRequest(expectedDefaultKeyId: expectation.defaultKeyID,
                                                         expectedSecretsRevision: expectation.secretsRevision)
        let revision: UInt64?
        do {
            revision = try await api.createSecretStorageKey(body, reauth: reauth)
        } catch let failure as E2eeAPIError {
            guard failure.isSsssConflict else { throw Self.mapReauth(failure) }
            // 応答をなくして送り直したか、別の端末が先にリセットした。読み直して判定する。
            let current = try await api.secretStorage()
            guard try jsonObject(current).string("default_key_id") == session.keyId() else { throw E2eeSetupError.conflict }
            revision = Self.secretsRevision(current)
        }
        // SSSS を新しい鍵に作り直した。この先で止まっても、新しいリカバリーキーで解錠して、リセットの続きから終えられるよう、印を残す。
        try await store().setResetKeyID(session.keyId())
        try await saveRemaining(session, secretsRevision: revision, reauth: reauth, reset: true)
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
    func saveRemaining(_ session: SetupSession, secretsRevision: UInt64?, reauth: String?, reset: Bool = false) async throws {
        // リセットの途中で止まっていた鍵（SSSS だけ作り直し済み）を、解錠して続けるときも、リセットの続き（置き換える前の鍵が前提）を選ぶ。
        let resumingReset = try await isResetResume(session)
        let reset = reset || resumingReset
        let plan = try await planRemaining(session, reset: reset)
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
        if reset { try await store().setResetKeyID(nil) }
        // 解錠した（または初回設定を終えた）ので、手元のセッションを今の版に送り、過去のセッションを取り込む。
        // 失敗しても解錠は済んでいる（取り込みは、次の起動時の確認で続きを行う）。
        var ssssConflict = false
        do {
            // SSSS が書き換えられていたら（`E2EE_SSSS_CONFLICT`）、解錠できたとは言えない。通信の失敗とは分けて、呼び出し元へ返す。
            if case .conflict(let code) = try await uploadKeyBackup(), code == "E2EE_SSSS_CONFLICT" { ssssConflict = true }
            else if !session.isCreated() { _ = try await importKeyBackup() }
        } catch is CancellationError {
            throw CancellationError()
        } catch let failure as E2eeAPIError where failure.code == "E2EE_SSSS_CONFLICT" {
            ssssConflict = true
        } catch {}
        if ssssConflict { throw E2eeSetupError.secretsChanged }
    }

    /// 解錠した鍵が、リセットの途中で止まっていた新しい SSSS 鍵か。
    private func isResetResume(_ session: SetupSession) async throws -> Bool {
        try await store().resetKeyID == session.keyId()
    }

    private func planRemaining(_ session: SetupSession, reset: Bool = false) async throws -> ResumePlan {
        let server = try await serverState()
        let current = try await api.secretStorage()
        // 別の端末が SSSS を作り直した（同時の初回設定など）。手元の鍵でクロス署名鍵や版を差し替えると、
        // SSSS とサーバーの鍵の世代がずれるので、何も送らない（読んでから送るまでの間の競合は、
        // `expected_default_key_id` でサーバーが防ぐ）。
        guard try jsonObject(current).string("default_key_id") == session.keyId() else { throw E2eeSetupError.conflict }
        let deviceKeys = try await store().deviceKeys()
        do {
            if reset { return try session.planReset(keysQuery: server.keys, versions: server.versions, deviceKeys: deviceKeys) }
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
        let plan = try await planRemaining(session, reset: try await isResetResume(session))
        if plan.crossSigning != nil { return UnlockResult(session: session, secretsRevision: revision, needsReauth: true) }
        do {
            try await saveRemaining(session, secretsRevision: revision, reauth: nil)
        } catch E2eeSetupError.reauthRequired {
            // 過去の版（消した版を含む）があるユーザーが版を作り直すときも、直前の再認証が要る。呼び出し側が再認証して続ける。
            return UnlockResult(session: session, secretsRevision: revision, needsReauth: true)
        }
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
        if case .number(let value) = inner["secrets_revision"] { return UInt64(exactly: value) }
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

    func currentBackupVersion() async throws -> String? {
        do { return String(decoding: try await e2ee("api/e2ee/key_backup/versions/current", method: "GET"), as: UTF8.self) }
        catch let failure as E2eeAPIError where failure.status == 404 { return nil }
    }

    func backupKeys(version: String, after: String?) async throws -> String {
        var query = ["version": version]
        if let after { query["after"] = after }
        return String(decoding: try await e2ee("api/e2ee/key_backup/keys", method: "GET", query: query), as: UTF8.self)
    }

    func uploadBackupKeys(version: String, secretsRevision: UInt64, body: String, proof: String?) async throws {
        _ = try await e2ee("api/e2ee/key_backup/keys", method: "POST",
                           query: ["version": version, "secrets_revision": String(secretsRevision)], body: jsonObject(body),
                           headers: proof.map { ["X-Zoo-Backup-Proof": $0] } ?? [:])
    }
}
