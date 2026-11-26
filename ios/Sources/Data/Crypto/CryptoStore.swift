import CryptoKit
import Foundation
import Synchronization
import ZooCrypto

/// 暗号ストアの置き場所。接続先とユーザーごとに分ける（docs/e2ee.md の「暗号ストア」、Android の `CryptoStoreFiles` と同じ）。
struct CryptoStoreFiles: Sendable {
    /// `state.json`（暗号ストアの状態）を置くディレクトリー。
    let directory: URL
    /// 保存鍵の置き場。
    let keys: StorageKeyStore

    /// 接続先 `server` のユーザー `userID` の暗号ストア。ID は Android と同じ `SHA-256("<接続先>\n<ユーザー>")` の先頭 32 文字。
    static func ios(root: URL, server: String, userID: String) -> CryptoStoreFiles {
        let id = storeID(server: server, userID: userID)
        return CryptoStoreFiles(directory: root.appendingPathComponent(id, isDirectory: true),
                                keys: StorageKeyStore(backing: KeychainStorageKey(account: id)))
    }

    static func storeID(server: String, userID: String) -> String {
        SHA256.hash(data: Data("\(server)\n\(userID)".utf8)).map { String(format: "%02x", $0) }.joined().prefix(32).description
    }

    /// 暗号ストアの置き場。通知拡張が `state.json` を読めるよう、App Group（`group.works.zli.zoo`）のコンテナーに置く。
    /// App Group を使えない環境（署名のない単体テストなど）では Application Support に置く。どちらも iCloud バックアップと
    /// 端末間の転送には入れない。以前の置き場（Application Support）に暗号ストアがあれば、App Group に移す。
    static func defaultRoot() throws -> URL {
        let support = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("e2ee", isDirectory: true)
        var root = support
        if let group = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: NotificationE2ee.appGroup) {
            root = group.appendingPathComponent("e2ee", isDirectory: true)
        }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        if root != support, let old = try? FileManager.default.contentsOfDirectory(at: support, includingPropertiesForKeys: nil) {
            for directory in old {
                let target = root.appendingPathComponent(directory.lastPathComponent)
                if !FileManager.default.fileExists(atPath: target.path) { try? FileManager.default.moveItem(at: directory, to: target) }
            }
        }
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try root.setResourceValues(values)
        return root
    }
}

/// 送信に解錠が要る（解錠していない、解錠し直しが要る、サーバーの自己署名鍵が暗号ストアの鍵と違う）。
enum E2eeStoreError: Error, Equatable {
    case sendNeedsUnlock
    /// 閉じた（または消した）暗号ストアは使えない。
    case closed
    /// 送信者の鍵（端末）が、セッションを作ったときと変わった。そのセッションでは暗号化できない。
    case senderChanged
    /// 終わっていない失効がある（黙って捨てない）。
    case revocationPending
    /// 続ける失効がない。
    case noRevocation
}

/// 同じ暗号ストアを、ほかの場所が開いている。少し待って試し直す。
struct CryptoStoreBusyError: Error, Equatable {}

/// `state.json` の中身。`account` は `DeviceAccount.pickle`（保存鍵で暗号化済み）。
/// `user_id`、`device_id`、登録済みかは平文で、秘密ではない（Android・TUI の `state.json` と同じ形）。
struct StoreState: Codable {
    var version: Int
    var user_id: String
    var device_id: String
    var registered: Bool
    var account: String
    /// 配布先の判定の記録（`TrustStore::pickle`）。
    var trust_store: String?
    /// 解錠した（または初回設定で作った）自分の鍵（`OwnKeys` の封筒。マスター鍵の秘密鍵は入れない）。
    var own_keys: String?
    /// 最後に解錠したときの `secrets_revision`。
    var secrets_revision: UInt64?
    /// Olm セッション（`OlmSessionStore::pickle_sessions`）。
    var olm_sessions: [StoredOlm] = []
    /// 受信用セッション。キーは `inboundKey`。
    var inbound_sessions: [String: StoredInbound] = [:]
    /// 処理を終えて、まだ ack を送れていない to-device の ID。
    var processed_to_device: [String] = []
    /// 保存鍵を失って作り直した前の端末のうち、サーバーから消せていないもの。新しい状態と一緒に保存するので、
    /// 開き直しても失わない。
    var pending_deletions: [String] = []
    /// 鍵バックアップの送信の状態（`BackupUploadState.to_json`。秘密は入らない）。
    var backup_upload: String?
    /// 解錠したあと、過去のセッションを最後まで取り込んだ鍵バックアップの版（新しい端末での解錠の手順 5）。
    var imported_backup_version: String?
    /// 終わっていない失効の進み具合（`Revocation.pickle`。マスター鍵の秘密鍵と SSSS 鍵は入らない）。
    var revocation: String?
    /// 暗号化チャンネルとして見たことがあるチャンネルの ID（サーバーが後から `encrypted = false` と返しても、格下げとして扱う）。
    var encrypted_channels: [String] = []

    init(version: Int, user_id: String, device_id: String, registered: Bool, account: String, trust_store: String?) {
        self.version = version; self.user_id = user_id; self.device_id = device_id
        self.registered = registered; self.account = account; self.trust_store = trust_store
    }

    // 後から足した項目は、ないときの既定値で読む（前の版の `state.json` を開けるように）。
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        version = try c.decode(Int.self, forKey: .version)
        user_id = try c.decode(String.self, forKey: .user_id)
        device_id = try c.decode(String.self, forKey: .device_id)
        registered = try c.decode(Bool.self, forKey: .registered)
        account = try c.decode(String.self, forKey: .account)
        trust_store = try c.decodeIfPresent(String.self, forKey: .trust_store)
        own_keys = try c.decodeIfPresent(String.self, forKey: .own_keys)
        secrets_revision = try c.decodeIfPresent(UInt64.self, forKey: .secrets_revision)
        olm_sessions = try c.decodeIfPresent([StoredOlm].self, forKey: .olm_sessions) ?? []
        inbound_sessions = try c.decodeIfPresent([String: StoredInbound].self, forKey: .inbound_sessions) ?? [:]
        processed_to_device = try c.decodeIfPresent([String].self, forKey: .processed_to_device) ?? []
        pending_deletions = try c.decodeIfPresent([String].self, forKey: .pending_deletions) ?? []
        backup_upload = try c.decodeIfPresent(String.self, forKey: .backup_upload)
        imported_backup_version = try c.decodeIfPresent(String.self, forKey: .imported_backup_version)
        revocation = try c.decodeIfPresent(String.self, forKey: .revocation)
        encrypted_channels = try c.decodeIfPresent([String].self, forKey: .encrypted_channels) ?? []
    }
}

struct StoredOlm: Codable {
    var their_identity_key: String
    var session_id: String
    var pickle: String
}

struct StoredInbound: Codable, Equatable {
    var pickle: String
    /// 鍵バックアップに送ったか。送っていないものの数を、ログアウトの前に見せる。
    var backed_up: Bool = false
}

/// iOS の暗号ストア（#685）。今は端末アカウントと登録の状態だけを持ち、Olm・Megolm のセッションなどは送受信の実装で足す。
///
/// - `state.json` は一時ファイルに書いて同期してから置き換えるので、書いている途中で終了しても、前の状態か
///   次の状態のどちらかが残る。保存に失敗したら例外を投げる。そのときメモリーの状態は保存より先に進んでいるので、
///   呼び出し側は閉じて開き直してから続ける。
/// - 同じ暗号ストアを同時に 2 つ開かない（[CryptoStoreBusyError]）。
final class CryptoStore: @unchecked Sendable {
    struct Opened {
        let store: CryptoStore
    }

    static let stateVersion = 1
    static let stateFile = "state.json"
    /// 開いている暗号ストアのディレクトリーと、その予約の持ち主（予約ごとの印）。解除は、持ち主の印が合うときだけ行う
    /// （失敗の後始末と `deinit` が重なっても、あとから開いた別のインスタンスの予約を外さない）。
    private static let openDirectories = Mutex([String: UUID]())

    private static func release(_ path: String, token: UUID) {
        openDirectories.withLock { if $0[path] == token { $0[path] = nil } }
    }

    /// 単体テスト用：このディレクトリーを、ほかの場所が予約しているか。
    static func isReserved(_ directory: URL) -> Bool {
        openDirectories.withLock { $0[directory.standardizedFileURL.path] != nil }
    }

    /// 単体テスト用：持ち主の印が違う解除は、何もしない。
    static func releaseReservation(_ directory: URL, token: UUID) {
        release(directory.standardizedFileURL.path, token: token)
    }

    private let reservation: UUID

    /// 単体テスト用：そのディレクトリーの `open` が失敗して予約を外す直前（作りかけのインスタンスの `deinit` のあと）に呼ぶ。
    /// ディレクトリーごとなので、並行して動くほかのテストの `open` には呼ばれない。
    static let openFailureHooks = Mutex<[String: @Sendable () -> Void]>([:])

    private let directory: URL
    /// 開いたときの `user_id`。暗号ストアの持ち主は変わらないので、ロックなしで読める。
    let userID: String
    private let storageKey: Data
    private var account: DeviceAccount
    private var trust: TrustStore
    private var olmSessions: OlmSessionStore
    private var inbound = InboundSessionStore()
    private var loadedInbound = Set<String>()
    private var outboundOlm: [String: String] = [:]
    /// 鍵バックアップの送信の状態。初めて使うときに `state.backup_upload` から読む。
    private var backupState: BackupUploadState?
    /// 送っている途中の要求と、送ったときのセッションの pickle（送ったあとに書き換わったものを送信待ちのまま残すため）。
    private struct PendingUpload { let upload: BackupUpload; let sessions: [(String, String)] }
    private var pendingUploads: [Int: PendingUpload] = [:]
    private var nextUploadID = 0
    private var state: StoreState
    private var closed = false
    private let lock = NSLock()

    var deviceID: String { lock.withLock { state.device_id } }
    /// 今の端末の Curve25519 鍵（端末を作り直すと変わる）。
    var curve25519Key: String { lock.withLock { account.curve25519Key() } }
    /// サーバーから消せていない前の端末の ID。
    var pendingDeviceDeletions: [String] { lock.withLock { state.pending_deletions } }
    var isRegistered: Bool { lock.withLock { state.registered } }

    private init(directory: URL, storageKey: Data, account: DeviceAccount, trust: TrustStore, olmSessions: OlmSessionStore,
                 state: StoreState, reservation: UUID) {
        self.directory = directory
        self.reservation = reservation
        self.userID = state.user_id
        self.olmSessions = olmSessions
        self.storageKey = storageKey
        self.account = account
        self.trust = trust
        self.state = state
    }

    deinit { close() }

    /// 開く。なければ、`create` のとき新しい端末を作る。保存鍵を開けなければ（Keychain から消えた、壊れた、状態と合わない）、
    /// 状態も読めないので、新しい端末を作る。前の端末の ID は新しい状態に入れて保存する（`pendingDeviceDeletions`）ので、
    /// 作り直しの途中で失敗しても、開き直すと取り出せる。Keychain の一時的な失敗はそのまま投げる。
    static func open(_ files: CryptoStoreFiles, userID: String, create: Bool = true) throws -> Opened? {
        let path = files.directory.standardizedFileURL.path
        let token = UUID()
        guard openDirectories.withLock({ guard $0[path] == nil else { return false }; $0[path] = token; return true }) else {
            throw CryptoStoreBusyError()
        }
        do {
            guard let opened = try openUnlocked(files, userID: userID, create: create, reservation: token) else {
                release(path, token: token)
                return nil
            }
            return opened
        } catch {
            // 作りかけのインスタンスの `deinit` が先に外していても、持ち主の印が合うときだけ外すので、重ならない。
            openFailureHooks.withLock { $0[path] }?()
            release(path, token: token)
            throw error
        }
    }

    private static func openUnlocked(_ files: CryptoStoreFiles, userID: String, create: Bool, reservation: UUID) throws -> Opened? {
        let stateURL = files.directory.appendingPathComponent(stateFile)
        var pending: [String] = []
        if let stored = try readState(stateURL) {
            guard stored.version == stateVersion else { throw CocoaError(.fileReadCorruptFile) }
            guard stored.user_id == userID else { throw CocoaError(.fileReadCorruptFile) }
            let key: Data?
            do { key = try files.keys.loadExisting() } catch StorageKeyError.malformed { key = nil }
            if let key {
                // 保存鍵が端末アカウントと合わないときだけ、作り直す（作り直しの途中で止まった、保存鍵だけ別のもの）。
                // 信頼の記録や Olm セッションの復元の失敗は、保存鍵のせいとは言えないので、作り直さずに投げる
                // （受信用セッションを失わず、前の端末も消さない）。
                let restored: DeviceAccount?
                do {
                    restored = try DeviceAccount.fromPickle(pickle: stored.account, storageKey: key,
                                                            userId: stored.user_id, deviceId: stored.device_id)
                } catch is ZooCryptoError {
                    restored = nil
                }
                if let account = restored {
                    let trust = try stored.trust_store.map { try TrustStore.fromPickle(pickle: $0, account: account, storageKey: key) }
                        ?? TrustStore(userId: stored.user_id)
                    let olm = OlmSessionStore()
                    for session in stored.olm_sessions { _ = try olm.restoreSession(account: account, pickle: session.pickle, storageKey: key) }
                    return Opened(store: CryptoStore(directory: files.directory, storageKey: key, account: account, trust: trust,
                                                     olmSessions: olm, state: stored, reservation: reservation))
                }
            }
            // 保存鍵がなければ、どの値も読めない。作り直して、新しい端末として登録し直す。前の端末の ID は、
            // 新しい状態（これが前の `state.json` を置き換える）に入れる。置き換える前に消さないので、
            // 途中で失敗しても、前の `state.json` から同じ ID を取り出せる。
            pending = stored.pending_deletions + [stored.device_id]
            guard create else { return nil }
            try files.keys.delete()
        }
        guard create else { return nil }
        try FileManager.default.createDirectory(at: files.directory, withIntermediateDirectories: true)
        let key: Data
        do { key = try files.keys.loadOrCreate() } catch StorageKeyError.malformed {
            try files.keys.delete()
            key = try files.keys.loadOrCreate()
        }
        let account = try DeviceAccount(userId: userID, deviceId: UUID().uuidString.lowercased())
        account.generateOneTimeKeys(count: account.maxNumberOfOneTimeKeys())
        account.generateFallbackKey()
        let trust = try TrustStore(userId: userID)
        var state = StoreState(version: stateVersion, user_id: userID, device_id: account.deviceId(),
                               registered: false, account: try account.pickle(storageKey: key),
                               trust_store: try trust.pickle(account: account, storageKey: key))
        state.pending_deletions = pending
        let store = CryptoStore(directory: files.directory, storageKey: key, account: account, trust: trust,
                                olmSessions: OlmSessionStore(), state: state, reservation: reservation)
        try store.save()
        return Opened(store: store)
    }

    /// `POST /api/e2ee/devices` の本文（`display_name` を除く）。応答をなくしても同じ鍵で送り直す。
    func registrationRequest() throws -> [String: JSONValue] {
        try lock.withLock {
            func parse(_ json: String) throws -> JSONValue { try JSONDecoder().decode(JSONValue.self, from: Data(json.utf8)) }
            return ["device_keys": try parse(account.deviceKeys()),
                    "one_time_keys": try parse(account.oneTimeKeys()),
                    "fallback_keys": try parse(account.fallbackKeys())]
        }
    }

    func markRegistered() throws {
        try lock.withLock {
            account.markKeysAsPublished()
            state.account = try account.pickle(storageKey: storageKey)
            state.registered = true
            try save()
        }
    }

    /// 別の端末から削除されたとき、暗号ストアは残したまま端末だけを新しい `device_id` に作り直す
    /// （削除済みの `device_id` は再利用できない）。送受信のセッションなどを持つようになったら、ここで封をし直す。
    func replaceDevice() throws {
        try lock.withLock {
            let next = try DeviceAccount(userId: state.user_id, deviceId: UUID().uuidString.lowercased())
            next.generateOneTimeKeys(count: next.maxNumberOfOneTimeKeys())
            next.generateFallbackKey()
            var nextState = state
            nextState.device_id = next.deviceId()
            nextState.registered = false
            // 自分の鍵と配布先の記録は古い端末アカウントで封をしてあるので、解錠し直すまで使わない。
            nextState.own_keys = nil
            nextState.secrets_revision = nil
            nextState.imported_backup_version = nil
            nextState.trust_store = try TrustStore(userId: state.user_id).pickle(account: next, storageKey: storageKey)
            nextState.account = try next.pickle(storageKey: storageKey)
            // Olm セッションは古い端末の鍵との間のものなので捨てる。受信用セッションは、古い端末で開いて新しい端末で封をし直す
            // （開けない、壊れたものは捨てる）。
            nextState.olm_sessions = []
            nextState.processed_to_device = []
            let opener = InboundSessionStore()
            var resealed: [String: StoredInbound] = [:]
            for (key, stored) in state.inbound_sessions {
                guard let ref = try? opener.restoreSession(account: account, pickle: stored.pickle, storageKey: storageKey),
                      let pickle = try? opener.pickleSession(account: next, key: ref, storageKey: storageKey) else { continue }
                resealed[key] = StoredInbound(pickle: pickle, backed_up: stored.backed_up)
            }
            nextState.inbound_sessions = resealed
            try save(nextState)
            // 保存できてから、メモリーの状態を進める。
            account = next
            trust = try TrustStore.fromPickle(pickle: nextState.trust_store!, account: next, storageKey: storageKey)
            olmSessions = OlmSessionStore()
            inbound = InboundSessionStore()
            loadedInbound = []
            outboundOlm = [:]
            state = nextState
        }
    }

    /// 今の端末の署名付きの端末鍵（JSON）。
    func deviceKeys() -> String { lock.withLock { account.deviceKeys() } }

    /// 自分の鍵を保存してあるか（解錠済みの端末か）。
    var hasOwnKeys: Bool { lock.withLock { state.own_keys != nil } }

    /// 最後に解錠したときの `secrets_revision`。
    var secretsRevision: UInt64? { lock.withLock { state.secrets_revision } }

    /// 解錠した（または初回設定で作った）自分の鍵を保存する。マスター鍵の秘密鍵は保存せず、自分のマスター鍵の
    /// 公開鍵とユーザー署名鍵を、配布先の判定の記録に入れる。書けなければ、メモリーの状態も前に戻して投げる。
    func saveOwnKeys(_ session: SetupSession, secretsRevision: UInt64?) throws {
        try lock.withLock {
            // 記録はコピーで更新し、すべて保存できたときだけ置き換える。
            let updated = try TrustStore.fromJson(json: trust.toJson(), userId: state.user_id)
            try session.recordOwnKeys(trust: updated)
            var next = state
            next.own_keys = try session.ownKeysPickle(account: account, storageKey: storageKey)
            next.secrets_revision = secretsRevision
            next.trust_store = try updated.pickle(account: account, storageKey: storageKey)
            // 解錠し直したら、鍵バックアップから取り込み直す。
            next.imported_backup_version = nil
            try save(next)
            state = next
            trust = updated
            // 新しい秘密を記録したので、鍵バックアップの今の版は検証し直す。
            backupState?.suspend()
        }
    }

    /// 起動時にすることを、暗号ストアの自分の鍵とサーバーの状態（自分だけの `keys/query` と
    /// `GET /api/e2ee/key_backup/versions` の応答）から決める。まだ解錠していなければ `nil`。
    func startupCheck(keysQuery: String, versions: String) throws -> StartupCheck? {
        try lock.withLock {
            try ensureOpen()
            guard let sealed = state.own_keys else { return nil }
            return try OwnKeys.fromPickle(sealed: sealed, account: account, storageKey: storageKey)
                .startupCheck(keysQuery: keysQuery, versions: versions, deviceKeys: account.deviceKeys())
        }
    }

    /// 解錠し直すまで、暗号ストアの鍵を使う処理を止める。`failed` は、409 になった要求の `secrets_revision`。
    /// 送っている間に解錠し直して新しい値を保存していたら（値が違えば）、その解錠を取り消さない。
    func forgetSecretsRevision(_ failed: UInt64) throws {
        try lock.withLock {
            guard state.secrets_revision == failed else { return }
            backupState?.setSecretsRevision(secretsRevision: nil)
            backupState?.suspend()
            var next = state
            next.secrets_revision = nil
            try save(next)
            state = next
        }
    }

    // MARK: - ワンタイム鍵

    /// `POST /api/e2ee/keys/upload` に送る鍵。
    struct KeyUpload: Equatable {
        let oneTimeKeys: [String: JSONValue]
        let fallbackKeys: [String: JSONValue]
    }

    /// サーバーのワンタイム鍵を最大数に戻し、使われたフォールバック鍵を作り直す。送るものがなければ `nil`。
    /// まだ公開していない鍵（応答をなくした送信の分）も入れる。
    func prepareKeyUpload(oneTimeKeyCount: Int, hasUnusedFallbackKey: Bool) throws -> KeyUpload? {
        try lock.withLock {
            func parse(_ json: String) throws -> [String: JSONValue] {
                guard case .object(let object) = try JSONDecoder().decode(JSONValue.self, from: Data(json.utf8)) else { throw ZooError.invalidResponse }
                return object
            }
            let unpublished = try parse(account.oneTimeKeys()).count
            let missing = Int(account.maxNumberOfOneTimeKeys()) - oneTimeKeyCount - unpublished
            var changed = false
            if missing > 0 { account.generateOneTimeKeys(count: UInt64(missing)); changed = true }
            if !hasUnusedFallbackKey, try parse(account.fallbackKeys()).isEmpty { account.generateFallbackKey(); changed = true }
            if changed {
                var next = state
                next.account = try account.pickle(storageKey: storageKey)
                try save(next)
                state = next
            }
            let oneTimeKeys = try parse(account.oneTimeKeys()), fallbackKeys = try parse(account.fallbackKeys())
            if oneTimeKeys.isEmpty && fallbackKeys.isEmpty { return nil }
            return KeyUpload(oneTimeKeys: oneTimeKeys, fallbackKeys: fallbackKeys)
        }
    }

    func markKeysPublished() throws {
        try lock.withLock {
            account.markKeysAsPublished()
            var next = state
            next.account = try account.pickle(storageKey: storageKey)
            try save(next)
            state = next
        }
    }

    // MARK: - 受信（to-device、本文の復号）

    /// 受信用セッションのキーを 1 つの文字列にする（Android・TUI と同じ形）。
    static func inboundKey(_ ref: SessionRef) -> String {
        String(decoding: try! JSONEncoder().encode([ref.channelId, ref.senderKey, ref.sessionId]), as: UTF8.self)
    }

    /// 閉じたストアには書かない（閉じたあとに、古いインスタンスが開き直したストアの状態を書き換えないため）。
    private func ensureOpen() throws { if closed { throw E2eeStoreError.closed } }

    private func save(_ next: StoreState) throws {
        try ensureOpen()
        try Self.writeAtomically(JSONEncoder().encode(next), to: directory.appendingPathComponent(Self.stateFile))
    }

    /// 保存した受信用セッションを、まだなら読み込む。読み込めたときだけ読み込み済みにする。
    private func loadInbound(_ key: String) throws {
        guard !loadedInbound.contains(key) else { return }
        // 保存していないセッションは、覚えない（鍵が届かない別のセッションを読み続けても、メモリーが増えない）。
        guard let stored = state.inbound_sessions[key] else { return }
        _ = try inbound.restoreSession(account: account, pickle: stored.pickle, storageKey: storageKey)
        loadedInbound.insert(key)
    }

    private func saveTrustStore() throws {
        var next = state
        next.trust_store = try trust.pickle(account: account, storageKey: storageKey)
        try save(next)
        state = next
    }

    /// to-device の Shape の行のうち、処理に使う列。
    struct ToDeviceRow: Equatable {
        let id: String
        let senderUserID: String
        let type: String
        let content: String
    }

    struct ToDeviceResult: Equatable {
        /// 処理を終えた（保存した、または捨てた）ID。ack を送ってよい。
        let processed: [String]
        let roomKeys: Int
        let dropped: Int
        /// 新しく作った Olm セッションの数（ワンタイム鍵を使われたので補充する）。
        let newOlmSessions: Int
        /// 保存した鍵のうち、送信端末を検証できたものの数。
        let verifiedRoomKeys: Int
        /// 受信時の検証を通った、SAS の検証のメッセージ（JSON。`SasVerification` に渡す）。
        var verifications: [String] = []
    }

    /// to-device メッセージを処理する（Android の `CryptoStore.processToDevice`、TUI・React と同じ規則）。
    ///
    /// 1 件ごとに、変わった状態と処理済みの印を保存してから次に進む。ack の前に終了しても、同じメッセージを 2 回
    /// 復号しない。`keysQuery` は送信ユーザーと自分の `keys/query` の応答（端末鍵と署名を含む）で、解錠していれば、
    /// 自分と送信者のクロス署名と保存済みの信頼記録で送信端末を検証する。
    func processToDevice(_ rows: [ToDeviceRow], keysQuery: String) throws -> ToDeviceResult {
        try lock.withLock {
            try ensureOpen()
            var processed: [String] = [], roomKeys = 0, dropped = 0, newOlm = 0, verified = 0, verifications: [String] = []
            let query = (try? JSONDecoder().decode(JSONValue.self, from: Data(keysQuery.utf8))) ?? .null
            // 前の処理で印を付けた行（ack の前に失敗した）。保存の同期が済んだとは限らないので、返す前に保存し直す。
            var unsaved = false
            for row in rows {
                if state.processed_to_device.contains(row.id) {
                    unsaved = true
                } else {
                    if row.type == "m.room.encrypted" {
                        switch try processEncrypted(row, query: query, keysQuery: keysQuery) {
                        case .dropped: dropped += 1
                        case .verification(let created, let message):
                            newOlm += created
                            if let message { verifications.append(message) } else { dropped += 1 }
                        case .decrypted(let created, let roomKey, let rejected, let isVerified):
                            newOlm += created
                            if roomKey { roomKeys += 1 } else if rejected { dropped += 1 }
                            if roomKey && isVerified { verified += 1 }
                        }
                    } else {
                        dropped += 1
                    }
                    state.processed_to_device.append(row.id)
                    try save(state)
                    unsaved = false
                }
                processed.append(row.id)
            }
            if unsaved { try save(state) }
            return ToDeviceResult(processed: processed, roomKeys: roomKeys, dropped: dropped, newOlmSessions: newOlm,
                                  verifiedRoomKeys: verified, verifications: verifications)
        }
    }

    private enum Encrypted {
        case dropped
        /// SAS の検証のメッセージ（検証を通らなければ `message` は `nil`）。
        case verification(newOlmSessions: Int, message: String?)
        case decrypted(newOlmSessions: Int, roomKey: Bool, rejected: Bool, verified: Bool)
    }

    private func ownSelfSigning() -> String? {
        // 解錠し直しが要る（secrets_revision を消した）ときは、暗号ストアの自分の鍵を使わない。
        guard let sealed = state.own_keys, state.secrets_revision != nil else { return nil }
        return try? OwnKeys.fromPickle(sealed: sealed, account: account, storageKey: storageKey).selfSigningPublicKey()
    }

    private func processEncrypted(_ row: ToDeviceRow, query: JSONValue, keysQuery: String) throws -> Encrypted {
        let before = olmSessions.len()
        let decrypted: OlmDecrypted
        do { decrypted = try account.decryptToDevice(sessions: olmSessions, content: row.content) } catch is ZooCryptoError {
            // 復号できないメッセージは何度試しても読めない。捨てて ack する。
            return .dropped
        }
        // 復号できたら、セッションの状態が進んでいる。検証の結果に関わらず保存する。
        let created = Int(olmSessions.len() - before)
        state.account = try account.pickle(storageKey: storageKey)
        let senderKey = decrypted.senderKey()
        let changed = try olmSessions.pickleSessions(account: account, storageKey: storageKey, theirIdentityKey: senderKey)
        state.olm_sessions.removeAll { stored in
            changed.contains { $0.sessionId == stored.session_id && $0.theirIdentityKey == stored.their_identity_key }
        }
        state.olm_sessions += changed.map { StoredOlm(their_identity_key: $0.theirIdentityKey, session_id: $0.sessionId, pickle: $0.pickle) }
        var listed: String?
        if let user = decrypted.sender(), let device = decrypted.senderDevice(),
           case .object(let root) = query, case .object(let devices) = root["device_keys"],
           case .object(let ofUser) = devices[user], let keys = ofUser[device] {
            listed = String(decoding: try JSONEncoder().encode(keys), as: UTF8.self)
        }
        if decrypted.eventType()?.hasPrefix("m.key.verification.") == true {
            // 解錠していない端末は、検証を受けない（相手のクロス署名をたどれない）。
            let message = state.secrets_revision == nil ? nil : try? decrypted.verifyVerification(account: account, listedDeviceKeys: listed)
            return .verification(newOlmSessions: created, message: message)
        }
        var trustResult: SenderTrust?
        if let user = decrypted.sender(), let device = decrypted.senderDevice(), state.secrets_revision != nil {
            trustResult = try trust.senderTrust(keysQuery: keysQuery, sender: user, senderDevice: device, ownSelfSigning: ownSelfSigning())
            // Olm の更新・受信用鍵・処理済み ID と同じ行単位の保存に含める。
            state.trust_store = try trust.pickle(account: account, storageKey: storageKey)
        }
        let roomKey: ReceivedRoomKey
        do { roomKey = try decrypted.verifyRoomKey(account: account, listedDeviceKeys: listed, trust: trustResult) } catch is ZooCryptoError {
            return .decrypted(newOlmSessions: created, roomKey: false, rejected: true, verified: false)
        }
        // addRoomKey で鍵を渡す前に読む。
        let verified = roomKey.senderVerified()
        let ref = SessionRef(channelId: roomKey.channelId(), senderKey: roomKey.senderKey(), sessionId: roomKey.sessionId())
        let key = Self.inboundKey(ref)
        // 読み込めたときだけ読み込み済みにする（失敗したら次の試行で読み直す）。
        try loadInbound(key)
        if try inbound.addRoomKey(roomKey: roomKey) == .kept {
            return .decrypted(newOlmSessions: created, roomKey: false, rejected: false, verified: false)
        }
        guard let pickle = try inbound.pickleSession(account: account, key: ref, storageKey: storageKey) else {
            return .decrypted(newOlmSessions: created, roomKey: false, rejected: false, verified: false)
        }
        state.inbound_sessions[key] = StoredInbound(pickle: pickle, backed_up: false)
        loadedInbound.insert(key)
        return .decrypted(newOlmSessions: created, roomKey: true, rejected: false, verified: verified)
    }

    /// ack を送れた ID を、処理済みの印から外す。
    func acknowledged(_ ids: [String]) throws {
        try lock.withLock {
            guard !ids.isEmpty else { return }
            var next = state
            next.processed_to_device.removeAll { ids.contains($0) }
            try save(next)
            state = next
        }
    }

    /// 鍵バックアップにまだ送っていない受信用セッションの数。
    func pendingBackupCount() -> Int { lock.withLock { state.inbound_sessions.values.filter { !$0.backed_up }.count } }

    /// 保存している受信用セッションの数。
    var inboundSessionCount: Int { lock.withLock { state.inbound_sessions.count } }

    /// 本文を復号する。channel・id・parent・sender・epoch・revision とリプレイの照合は、共通の暗号基盤で行う。
    func decrypt(_ message: EncryptedMessage) throws -> DecryptedMessage {
        try lock.withLock {
            try ensureOpen()
            guard case .object(let envelope) = try JSONDecoder().decode(JSONValue.self, from: Data(message.ciphertext.utf8)),
                  case .string(let senderKey) = envelope["sender_key"], case .string(let sessionID) = envelope["session_id"] else {
                throw ZooError.invalidResponse
            }
            try loadInbound(Self.inboundKey(SessionRef(channelId: message.channelId, senderKey: senderKey, sessionId: sessionID)))
            return try inbound.decrypt(message: message)
        }
    }

    // MARK: - 暗号化チャンネルの記録（格下げの検出）

    /// 暗号化チャンネルとして見たチャンネルを記録する（新しいものがあるときだけ保存する）。
    func rememberEncryptedChannels(_ ids: [String]) throws {
        try lock.withLock {
            try ensureOpen()
            let known = Set(state.encrypted_channels)
            let added = ids.filter { !known.contains($0) }
            guard !added.isEmpty else { return }
            var next = state
            next.encrypted_channels += Set(added).sorted()
            try save(next)
            state = next
        }
    }

    func isRememberedEncrypted(_ id: String) -> Bool { lock.withLock { state.encrypted_channels.contains(id) } }

    // MARK: - 失効（#362）

    /// 保存した失効の進み具合（秘密鍵を除く表示用の JSON。なければ `nil`）。
    func revocationSummary() throws -> String? {
        try lock.withLock {
            try ensureOpen()
            guard let sealed = state.revocation else { return nil }
            return try peekRevocation(sealed: sealed, account: account, storageKey: storageKey)
        }
    }

    /// 失効を始める。終わっていない失効があれば `E2eeStoreError.revocationPending`（黙って捨てない）。解錠した SSSS が前提。
    func beginRevocation(input: String, unlocked: UnlockedSecretStorage) throws -> Revocation {
        try lock.withLock {
            try ensureOpen()
            guard state.revocation == nil else { throw E2eeStoreError.revocationPending }
            var backup: BackupDecryptionKey?
            var userSigning: CrossSigningKeyPair?
            if state.secrets_revision != nil, let sealed = state.own_keys {
                let own = try OwnKeys.fromPickle(sealed: sealed, account: account, storageKey: storageKey)
                backup = own.backupKey()
                userSigning = own.userSigning()
            }
            let revocation = try ZooCrypto.beginRevocation(inputJson: input, unlocked: unlocked, trust: trust,
                                                           localBackupKey: backup, oldUserSigning: userSigning)
            try saveRevocationLocked(revocation)
            return revocation
        }
    }

    /// 保存した失効を、開け直した SSSS で続ける。
    func resumeRevocation(unlocked: UnlockedSecretStorage) throws -> Revocation {
        try lock.withLock {
            try ensureOpen()
            guard let sealed = state.revocation else { throw E2eeStoreError.noRevocation }
            return try ZooCrypto.resumeRevocation(sealed: sealed, account: account, storageKey: storageKey, unlocked: unlocked)
        }
    }

    /// 進み具合を保存する（保存が 1 つ済むたびに）。
    func saveRevocation(_ revocation: Revocation) throws {
        try lock.withLock {
            try ensureOpen()
            try saveRevocationLocked(revocation)
        }
    }

    private func saveRevocationLocked(_ revocation: Revocation) throws {
        var next = state
        next.revocation = try revocation.pickle(account: account, storageKey: storageKey)
        try save(next)
        state = next
    }

    /// 失効の記録を消す（読み直して確かめたあと、またはやり直すとき）。
    func clearRevocation() throws {
        try lock.withLock {
            try ensureOpen()
            var next = state
            next.revocation = nil
            try save(next)
            state = next
        }
    }

    /// クロス署名鍵を保存したあと、記録と自分の鍵を新しい鍵にそろえ、新しい `secrets_revision` を記録する。
    func applyRevocation(_ revocation: Revocation, secretsRevision: UInt64?) throws {
        try lock.withLock {
            try ensureOpen()
            try revocation.applyToTrust(trust: trust)
            var next = state
            next.own_keys = try revocation.ownKeysPickle(account: account, storageKey: storageKey)
            next.secrets_revision = secretsRevision
            next.revocation = try revocation.pickle(account: account, storageKey: storageKey)
            next.trust_store = try trust.pickle(account: account, storageKey: storageKey)
            try save(next)
            state = next
        }
    }

    /// この端末の端末鍵（JSON。失効の署名に使う）。
    func deviceKeysJSON() -> String { lock.withLock { account.deviceKeys() } }

    // MARK: - 他のユーザーの検証（SAS、#368）

    /// `theirUserID` に検証を求める。`snapshotPages` は相手の `keys/query` の応答（JSON）。
    func sasRequest(snapshotPages: [String], theirUserID: String, nowMs: UInt64) throws -> SasRequest {
        try lock.withLock {
            try ensureOpen()
            guard state.secrets_revision != nil else { throw E2eeStoreError.sendNeedsUnlock }
            return try requestSasVerification(account: account, trust: trust, snapshotPages: snapshotPages,
                                              theirUserId: theirUserID, nowMs: nowMs)
        }
    }

    /// 届いた要求から検証を作る。
    func sasFromRequest(message: String, snapshotPages: [String], nowMs: UInt64) throws -> SasVerification {
        try lock.withLock {
            try ensureOpen()
            guard state.secrets_revision != nil else { throw E2eeStoreError.sendNeedsUnlock }
            return try SasVerification.fromRequest(account: account, trust: trust, messageJson: message,
                                                   snapshotPages: snapshotPages, nowMs: nowMs)
        }
    }

    /// 検証できた相手のマスター鍵に、自分のユーザー署名鍵で署名する（記録は登録待ちで置き換わる。保存する）。
    func signSasMasterKey(_ sas: SasVerification, masterKey: String) throws -> AcceptedMasterKey {
        try lock.withLock {
            try ensureOpen()
            guard state.secrets_revision != nil, let sealed = state.own_keys else { throw E2eeStoreError.sendNeedsUnlock }
            let own = try OwnKeys.fromPickle(sealed: sealed, account: account, storageKey: storageKey)
            let accepted = try sas.signMasterKey(trust: trust, userSigning: own.userSigning(), masterKey: masterKey)
            try saveTrustStore()
            return accepted
        }
    }

    /// 利用者が相手の今のマスター鍵（JSON）を受け入れる（再検証）。記録を登録待ちで置き換え、送るピンと署名を返す（保存する）。
    func acceptMasterKey(userID: String, masterKey: String) throws -> AcceptedMasterKey {
        try lock.withLock {
            try ensureOpen()
            guard state.secrets_revision != nil, let sealed = state.own_keys else { throw E2eeStoreError.sendNeedsUnlock }
            let own = try OwnKeys.fromPickle(sealed: sealed, account: account, storageKey: storageKey)
            let accepted = try trust.acceptMasterKey(userSigning: own.userSigning(), targetUserId: userID, masterKey: masterKey)
            try saveTrustStore()
            return accepted
        }
    }

    /// 受け入れのピンと署名をサーバーに保存できた（登録待ちを外す）。
    func confirmAcceptedMasterKey(userID: String, masterKey: String) throws {
        try lock.withLock {
            try ensureOpen()
            trust.confirmAcceptedMasterKey(targetUserId: userID, masterKey: masterKey)
            try saveTrustStore()
        }
    }

    /// 検証のメッセージを、`device` あての to-device の `content`（JSON）にする。Olm のセッションがなく、
    /// `claimed`（`keys/claim` のワンタイム鍵）もなければ `nil`。
    func encryptVerification(_ device: VerifiedDeviceKeys, eventType: String, content: String, claimed: String?) throws -> String? {
        try lock.withLock {
            try ensureOpen()
            guard state.secrets_revision != nil else { throw E2eeStoreError.sendNeedsUnlock }
            let curve = device.curve25519Key()
            let session: String
            if let existing = outboundOlm[curve] {
                session = existing
            } else if let claimed {
                session = try account.createOutboundOlmSession(sessions: olmSessions, recipient: device, oneTimeKey: claimed)
                outboundOlm[curve] = session
            } else {
                return nil
            }
            let encrypted = try account.encryptVerification(sessions: olmSessions, sessionId: session, recipient: device,
                                                            eventType: eventType, content: content)
            let changed = try olmSessions.pickleSessions(account: account, storageKey: storageKey, theirIdentityKey: curve)
            var next = state
            next.olm_sessions.removeAll { $0.their_identity_key == curve }
            next.olm_sessions += changed.map { StoredOlm(their_identity_key: $0.theirIdentityKey, session_id: $0.sessionId, pickle: $0.pickle) }
            next.account = try account.pickle(storageKey: storageKey)
            try save(next)
            state = next
            return encrypted
        }
    }

    // MARK: - 送信

    /// 解錠済みの鍵で配布先を判定し、TOFU の変更を保存する。
    func recipients(pages: [String]) throws -> Distribution {
        try lock.withLock {
            try ensureOpen()
            guard state.secrets_revision != nil, let sealed = state.own_keys else { throw E2eeStoreError.sendNeedsUnlock }
            let own = try OwnKeys.fromPickle(sealed: sealed, account: account, storageKey: storageKey)
            // 起動時の確認と送信が重なっても、失効前の自己署名鍵で送信を始めない。
            var matches = false
            for page in pages {
                guard case .object(let root) = try JSONDecoder().decode(JSONValue.self, from: Data(page.utf8)),
                      case .object(let selfSigning) = root["self_signing_keys"], case .object(let mine) = selfSigning[state.user_id],
                      case .object(let keys) = mine["keys"] else { continue }
                if keys.values.contains(.string(own.selfSigningPublicKey())) { matches = true; break }
            }
            guard matches else { throw E2eeStoreError.sendNeedsUnlock }
            let distribution = try trust.evaluate(snapshotPages: pages, ownDeviceId: state.device_id, userSigning: own.userSigning())
            try saveTrustStore()
            return distribution
        }
    }

    func confirmPin(_ pin: NewTofuPin, accepted: Bool) throws {
        try lock.withLock {
            if accepted { try trust.confirmTofuPin(pin: pin) } else { try trust.discardTofuPin(pin: pin) }
            try saveTrustStore()
        }
    }

    func needsOlm(_ device: VerifiedDeviceKeys) -> Bool { lock.withLock { outboundOlm[device.curve25519Key()] == nil } }

    /// Olm の状態を進めたら、ネットワークへ送る前に永続化する。`claimed` は `keys/claim` で受け取ったワンタイム鍵。
    func shareKey(_ device: VerifiedDeviceKeys, outbound: OutboundSession, claimed: String?) throws -> String? {
        try lock.withLock {
            try ensureOpen()
            guard state.secrets_revision != nil else { throw E2eeStoreError.sendNeedsUnlock }
            let curve = device.curve25519Key()
            let session: String
            if let existing = outboundOlm[curve] {
                session = existing
            } else if let claimed {
                session = try account.createOutboundOlmSession(sessions: olmSessions, recipient: device, oneTimeKey: claimed)
                outboundOlm[curve] = session
            } else {
                return nil
            }
            let encrypted = try account.encryptRoomKey(sessions: olmSessions, sessionId: session, recipient: device, outbound: outbound)
            let changed = try olmSessions.pickleSessions(account: account, storageKey: storageKey, theirIdentityKey: curve)
            var next = state
            next.olm_sessions.removeAll { $0.their_identity_key == curve }
            next.olm_sessions += changed.map { StoredOlm(their_identity_key: $0.theirIdentityKey, session_id: $0.sessionId, pickle: $0.pickle) }
            next.account = try account.pickle(storageKey: storageKey)
            try save(next)
            state = next
            return encrypted
        }
    }

    /// 本文を暗号化する。自分が送った本文も、再起動後に復号・バックアップできるよう、受信用の鍵を先に保存する。
    func encrypt(_ outbound: OutboundSession, senderKey: String, message: OutgoingMessage) throws -> String {
        try lock.withLock {
            try ensureOpen()
            // 鍵を配ったあとに端末が作り直されていたら、古い鍵で配ったセッションでは暗号化しない（確認と暗号化を同じロックで行う）。
            guard account.curve25519Key() == senderKey else { throw E2eeStoreError.senderChanged }
            guard state.secrets_revision != nil else { throw E2eeStoreError.sendNeedsUnlock }
            let ref = SessionRef(channelId: outbound.channelId(), senderKey: account.curve25519Key(), sessionId: outbound.sessionId())
            let key = Self.inboundKey(ref)
            if state.inbound_sessions[key] == nil {
                try loadInbound(key)
                _ = inbound.addOutbound(account: account, outbound: outbound)
                var next = state
                next.inbound_sessions[key] = StoredInbound(pickle: try inbound.pickleSession(account: account, key: ref, storageKey: storageKey)!)
                try save(next)
                state = next
                loadedInbound.insert(key)
            }
            return try outbound.encrypt(account: account, message: message)
        }
    }

    // MARK: - 鍵バックアップ

    /// `POST /api/e2ee/key_backup/keys` に送る要求。
    enum BackupUploadRequest: Equatable {
        /// 送るものがない（解錠していない、今の版がない、送信待ちがない）。
        case none
        /// 今の版を使えない（署名がない、SSSS の鍵と違う、版の巻き戻り、`upload_key` が鍵の形でないなど）。送らない。
        case rejected(String)
        /// 送る。`proof` は `X-Zoo-Backup-Proof` の値（今の版に `upload_key` がない、証明を足す前のサーバーなら `nil`）。
        case upload(id: Int, version: String, secretsRevision: UInt64, body: String, count: Int, proof: String?)
    }

    /// 受信用セッションのキー（`inboundKey`）の逆。
    static func sessionRef(_ key: String) throws -> SessionRef {
        guard let parts = try? JSONDecoder().decode([String].self, from: Data(key.utf8)), parts.count == 3 else {
            throw ZooError.invalidResponse
        }
        return SessionRef(channelId: parts[0], senderKey: parts[1], sessionId: parts[2])
    }

    private func loadBackupState() throws -> BackupUploadState {
        if let backupState { return backupState }
        let loaded = try state.backup_upload.flatMap { try? BackupUploadState.fromJson(json: $0, userId: state.user_id) }
        let value = try loaded ?? BackupUploadState(userId: state.user_id)
        backupState = value
        return value
    }

    /// サーバーが今の版として返した版（`GET /api/e2ee/key_backup/versions/current` の応答）を、暗号ストアの自分のマスター鍵と
    /// バックアップ鍵で検証して受け入れ、版の番号を返す。署名がない、公開鍵が違う、版が戻ったなどは `ZooCryptoError`。
    /// 版が変わったら、手元のセッションをすべて送信待ちに戻し、新しい版を見たことと同じ書き込みで記録する。
    private func acceptCurrentVersion(_ currentVersion: String) throws -> String {
        guard let sealed = state.own_keys else { throw E2eeStoreError.sendNeedsUnlock }
        let backup = try loadBackupState()
        backup.setSecretsRevision(secretsRevision: state.secrets_revision)
        let own = try OwnKeys.fromPickle(sealed: sealed, account: account, storageKey: storageKey)
        let change = try backup.acceptCurrentVersion(version: currentVersion, masterKey: own.masterPublicKey(), backupKey: own.backupKey())
        var next = state
        if change == .changed {
            next.inbound_sessions = next.inbound_sessions.mapValues { StoredInbound(pickle: $0.pickle, backed_up: false) }
        }
        next.backup_upload = backup.toJson()
        do { try save(next) } catch {
            // 新しい版を見たことを記録できなかったので、次の送信でもう一度戻す。
            backupState = nil
            throw error
        }
        state = next
        guard let version = backup.currentVersion()?.version else { throw ZooError.invalidResponse }
        return version
    }

    /// 鍵バックアップへ送る次の要求を作る（docs/e2ee.md の「鍵バックアップ」。TUI の `prepare_backup_upload`、
    /// Web の `prepareBackupUpload` と同じ規則）。`currentVersion` は今の版の応答（なければ `nil`）。今の版の `auth_data` に
    /// 自分のマスター鍵の署名があり、公開鍵が SSSS のバックアップ鍵と同じで、前に見た版より古くないときだけ送る。
    /// 版が変わったら、すべてのセッションを送り直す。
    func prepareBackupUpload(currentVersion: String?, limit: Int) throws -> BackupUploadRequest {
        try lock.withLock {
            try ensureOpen()
            // 送信は 1 つずつなので、ここで残っている要求は、送っている途中で捨てられたもの。記録を捨てる。
            pendingUploads.removeAll()
            guard state.own_keys != nil else { return .none }
            guard let currentVersion else {
                try loadBackupState().suspend()
                return .none
            }
            let version: String
            do { version = try acceptCurrentVersion(currentVersion) } catch let failure as ZooCryptoError {
                return .rejected(failure.localizedDescription)
            }
            let backup = try loadBackupState()
            let keys = Array(state.inbound_sessions.filter { !$0.value.backed_up }.keys.sorted().prefix(limit))
            guard !keys.isEmpty else { return .none }
            // 今の版に `upload_key` がなければ（証明を足す前のサーバー）、証明を付けない。あって文字列でなければ送らない
            // （証明を黙って省いて、未証明の行を送信済みにしないため）。
            var proof: String?
            if case .object(let object)? = try? JSONDecoder().decode(JSONValue.self, from: Data(currentVersion.utf8)),
               let uploadKey = object["upload_key"] {
                guard case .string(let key) = uploadKey else { return .rejected("upload_key が鍵の形ではありません") }
                guard let sealed = state.own_keys else { return .none }
                do {
                    proof = try OwnKeys.fromPickle(sealed: sealed, account: account, storageKey: storageKey).backupKey()
                        .uploadProof(uploadKey: key, userId: state.user_id, version: version)
                } catch is ZooCryptoError { return .rejected("upload_key が鍵の形ではありません") }
            }
            for key in keys {
                try loadInbound(key)
                backup.enqueue(session: try Self.sessionRef(key))
            }
            guard let upload = try backup.nextUpload(store: inbound, limit: UInt64(limit)) else { return .none }
            let sent = Set(upload.sessions().map(Self.inboundKey))
            let sessions = keys.filter { sent.contains($0) }.compactMap { key in state.inbound_sessions[key].map { (key, $0.pickle) } }
            nextUploadID += 1
            let id = nextUploadID
            pendingUploads[id] = PendingUpload(upload: upload, sessions: sessions)
            return .upload(id: id, version: upload.version(), secretsRevision: upload.secretsRevision(), body: upload.body(),
                           count: sessions.count, proof: proof)
        }
    }

    /// `id` の要求が成功した。その要求に入れたセッションだけを送信済みにする。
    func backupUploaded(_ id: Int) throws {
        try lock.withLock {
            guard let pending = pendingUploads.removeValue(forKey: id) else { return }
            var next = state
            if let backupState {
                backupState.uploadSucceeded(upload: pending.upload)
                next.backup_upload = backupState.toJson()
            }
            // 送ったあとに書き換わった（pickle が違う）セッションは、送信待ちのまま残す。
            for (key, pickle) in pending.sessions where next.inbound_sessions[key]?.pickle == pickle {
                next.inbound_sessions[key]?.backed_up = true
            }
            try save(next)
            state = next
        }
    }

    /// `id` の要求が失敗した。入れたセッションは送信待ちのまま残す。409 のときは、今の版を検証し直すまで送らない。
    func backupUploadFailed(_ id: Int, conflict: Bool) {
        lock.withLock {
            pendingUploads.removeValue(forKey: id)
            if conflict { backupState?.suspend() }
        }
    }

    /// 鍵バックアップから取り込む版を決める（新しい端末での解錠の手順 5）。解錠していない、今の版がない、この版から
    /// 取り込み終えているときは `nil`。版を使えない（`acceptCurrentVersion` が拒んだ）ときは `ZooCryptoError`。
    func backupVersionToImport(currentVersion: String?) throws -> String? {
        try lock.withLock {
            try ensureOpen()
            guard state.own_keys != nil, state.secrets_revision != nil, let currentVersion else { return nil }
            let version = try acceptCurrentVersion(currentVersion)
            return version == state.imported_backup_version ? nil : version
        }
    }

    /// `GET /api/e2ee/key_backup/keys` の 1 ページの行を、バックアップ鍵で復号して取り込む（docs/e2ee.md の「受信時の検証」の
    /// 「鍵バックアップから取り込んだとき」）。復号と照合は zoo-crypto が行い、Megolm のセッション鍵は Swift に出さない。
    /// バックアップ由来として保存し、Olm 由来のセッションを置き換えない。新しく取り込んだセッションは、この版から取ったので
    /// 送信済みにする。返すのは取り込めた行の数。
    func importBackupRows(_ rows: [String]) throws -> Int {
        try lock.withLock {
            try ensureOpen()
            guard let sealed = state.own_keys, !rows.isEmpty else { return 0 }
            var refs: [SessionRef] = []
            for row in rows {
                guard case .object(let object)? = try? JSONDecoder().decode(JSONValue.self, from: Data(row.utf8)),
                      case .string(let channel)? = object["channel_id"], case .string(let sender)? = object["sender_key"],
                      case .string(let session)? = object["session_id"] else { continue }
                refs.append(SessionRef(channelId: channel, senderKey: sender, sessionId: session))
            }
            for ref in refs { try loadInbound(Self.inboundKey(ref)) }
            let key = try OwnKeys.fromPickle(sealed: sealed, account: account, storageKey: storageKey).backupKey()
            let discarded = Int(try key.importRows(store: inbound, rows: rows))
            var next = state
            for ref in refs {
                let storeKey = Self.inboundKey(ref)
                guard let pickle = try inbound.pickleSession(account: account, key: ref, storageKey: storageKey) else { continue }
                let existing = next.inbound_sessions[storeKey]
                if existing?.pickle == pickle { continue }
                next.inbound_sessions[storeKey] = StoredInbound(pickle: pickle, backed_up: existing?.backed_up ?? true)
                loadedInbound.insert(storeKey)
            }
            if next.inbound_sessions != state.inbound_sessions { try save(next); state = next }
            return rows.count - discarded
        }
    }

    /// `version` の鍵バックアップから最後まで取り込んだ。
    func backupImported(_ version: String) throws {
        try lock.withLock {
            guard state.imported_backup_version != version else { return }
            var next = state
            next.imported_backup_version = version
            try save(next)
            state = next
        }
    }

    /// 暗号ストアを閉じる（同じ暗号ストアをまた開けるようにする）。
    func close() {
        lock.withLock {
            guard !closed else { return }
            closed = true
            Self.release(directory.standardizedFileURL.path, token: reservation)
        }
    }

    /// ログアウトで暗号ストアを消す。`state.json` を消し、保存鍵を Keychain から消す。消し終えるまで、同じ暗号ストアを
    /// ほかの場所が開けないようにする（予約を解除するのは、そのあと）。閉じたインスタンスからは消せない。
    func destroy(_ files: CryptoStoreFiles) throws {
        try lock.withLock {
            try ensureOpen()
            let stateURL = directory.appendingPathComponent(Self.stateFile)
            if FileManager.default.fileExists(atPath: stateURL.path) { try FileManager.default.removeItem(at: stateURL) }
            try files.keys.delete()
            closed = true
            Self.release(directory.standardizedFileURL.path, token: reservation)
        }
    }

    /// 前の端末をサーバーから消せたので、記録から外す。
    func clearPendingDeletion(_ deviceID: String) throws {
        try lock.withLock {
            var next = state
            next.pending_deletions.removeAll { $0 == deviceID }
            try save(next)
            state = next
        }
    }

    private func save() throws { try save(state) }

    private static func readState(_ url: URL) throws -> StoreState? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try JSONDecoder().decode(StoreState.self, from: Data(contentsOf: url))
    }

    /// 一時ファイルに書いて同期してから置き換える。失敗したら例外を投げる（保存できたことにして ack を送らないため）。
    /// 端末の初回解除後は通知拡張からも読めるよう、`completeUntilFirstUserAuthentication` にする。
    static func writeAtomically(_ data: Data, to url: URL) throws {
        let temporary = url.appendingPathExtension("tmp")
        try data.write(to: temporary, options: [.completeFileProtectionUntilFirstUserAuthentication])
        let handle = try FileHandle(forWritingTo: temporary)
        try handle.synchronize()
        try handle.close()
        // `rename(2)` は、同じボリュームで原子的に置き換える。失敗しても、元のファイルは動かさない（`replaceItemAt` は、
        // 失敗したときに元のファイルを別の場所へ移したままにすることがあり、前の端末の ID や受信用セッションの記録を
        // 取り出せなくなる）。
        guard rename(temporary.path, url.path) == 0 else {
            let code = errno
            try? FileManager.default.removeItem(at: temporary)
            throw POSIXError(POSIXErrorCode(rawValue: code) ?? .EIO)
        }
        // 置き換えたディレクトリーのエントリーも書き出す（停電で前の状態に戻らないように）。
        let directory = Darwin.open(url.deletingLastPathComponent().path, O_RDONLY)
        guard directory >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        defer { Darwin.close(directory) }
        guard fsync(directory) == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
    }
}
