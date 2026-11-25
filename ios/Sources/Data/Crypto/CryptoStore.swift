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

    /// 暗号ストアの置き場（Application Support。iCloud バックアップと端末間の転送には入れない）。
    static func defaultRoot() throws -> URL {
        var root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                               appropriateFor: nil, create: true)
            .appendingPathComponent("e2ee", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try root.setResourceValues(values)
        return root
    }
}

/// 送信に解錠が要る（解錠していない、解錠し直しが要る、サーバーの自己署名鍵が暗号ストアの鍵と違う）。
enum E2eeStoreError: Error, Equatable { case sendNeedsUnlock }

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
    }
}

struct StoredOlm: Codable {
    var their_identity_key: String
    var session_id: String
    var pickle: String
}

struct StoredInbound: Codable {
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
        /// 保存鍵を失って作り直したとき、前の端末の ID（サーバーから消す）。
        let replacedDeviceID: String?
    }

    static let stateVersion = 1
    static let stateFile = "state.json"
    private static let openDirectories = Mutex(Set<String>())

    private let directory: URL
    private let storageKey: Data
    private var account: DeviceAccount
    private var trust: TrustStore
    private var olmSessions: OlmSessionStore
    private var inbound = InboundSessionStore()
    private var loadedInbound = Set<String>()
    private var outboundOlm: [String: String] = [:]
    private var state: StoreState
    private var closed = false
    private let lock = NSLock()

    var userID: String { state.user_id }
    var deviceID: String { state.device_id }
    var isRegistered: Bool { lock.withLock { state.registered } }

    private init(directory: URL, storageKey: Data, account: DeviceAccount, trust: TrustStore, olmSessions: OlmSessionStore, state: StoreState) {
        self.directory = directory
        self.olmSessions = olmSessions
        self.storageKey = storageKey
        self.account = account
        self.trust = trust
        self.state = state
    }

    deinit { close() }

    /// 開く。なければ、`create` のとき新しい端末を作る。保存鍵を開けなければ（Keychain から消えた、壊れた）、
    /// 状態も読めないので、新しい端末を作って前の端末の ID を返す。Keychain の一時的な失敗はそのまま投げる。
    static func open(_ files: CryptoStoreFiles, userID: String, create: Bool = true) throws -> Opened? {
        let path = files.directory.standardizedFileURL.path
        guard openDirectories.withLock({ $0.insert(path).inserted }) else { throw CryptoStoreBusyError() }
        do {
            guard let opened = try openUnlocked(files, userID: userID, create: create) else {
                _ = openDirectories.withLock { $0.remove(path) }
                return nil
            }
            return opened
        } catch {
            _ = openDirectories.withLock { $0.remove(path) }
            throw error
        }
    }

    private static func openUnlocked(_ files: CryptoStoreFiles, userID: String, create: Bool) throws -> Opened? {
        let stateURL = files.directory.appendingPathComponent(stateFile)
        var replaced: String?
        if let stored = try readState(stateURL) {
            guard stored.version == stateVersion else { throw CocoaError(.fileReadCorruptFile) }
            guard stored.user_id == userID else { throw CocoaError(.fileReadCorruptFile) }
            let key: Data?
            do { key = try files.keys.loadExisting() } catch StorageKeyError.malformed { key = nil }
            if let key {
                let account = try DeviceAccount.fromPickle(pickle: stored.account, storageKey: key,
                                                           userId: stored.user_id, deviceId: stored.device_id)
                let trust = try stored.trust_store.map { try TrustStore.fromPickle(pickle: $0, account: account, storageKey: key) }
                    ?? TrustStore(userId: stored.user_id)
                let olm = OlmSessionStore()
                for session in stored.olm_sessions { _ = try olm.restoreSession(account: account, pickle: session.pickle, storageKey: key) }
                return Opened(store: CryptoStore(directory: files.directory, storageKey: key, account: account, trust: trust,
                                                 olmSessions: olm, state: stored),
                              replacedDeviceID: nil)
            }
            // 保存鍵がなければ、どの値も読めない。作り直して、新しい端末として登録し直す。
            replaced = stored.device_id
            try FileManager.default.removeItem(at: stateURL)
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
        let state = StoreState(version: stateVersion, user_id: userID, device_id: account.deviceId(),
                               registered: false, account: try account.pickle(storageKey: key),
                               trust_store: try trust.pickle(account: account, storageKey: key))
        let store = CryptoStore(directory: files.directory, storageKey: key, account: account, trust: trust,
                                olmSessions: OlmSessionStore(), state: state)
        try store.save()
        return Opened(store: store, replacedDeviceID: replaced)
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
            try Self.writeAtomically(JSONEncoder().encode(nextState), to: directory.appendingPathComponent(Self.stateFile))
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
            try Self.writeAtomically(JSONEncoder().encode(next), to: directory.appendingPathComponent(Self.stateFile))
            state = next
            trust = updated
        }
    }

    /// 起動時にすることを、暗号ストアの自分の鍵とサーバーの状態（自分だけの `keys/query` と
    /// `GET /api/e2ee/key_backup/versions` の応答）から決める。まだ解錠していなければ `nil`。
    func startupCheck(keysQuery: String, versions: String) throws -> StartupCheck? {
        try lock.withLock {
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
            var next = state
            next.secrets_revision = nil
            try Self.writeAtomically(JSONEncoder().encode(next), to: directory.appendingPathComponent(Self.stateFile))
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

    private func save(_ next: StoreState) throws {
        try Self.writeAtomically(JSONEncoder().encode(next), to: directory.appendingPathComponent(Self.stateFile))
    }

    /// 保存した受信用セッションを、まだなら読み込む。読み込めたときだけ読み込み済みにする。
    private func loadInbound(_ key: String) throws {
        guard !loadedInbound.contains(key) else { return }
        if let stored = state.inbound_sessions[key] {
            _ = try inbound.restoreSession(account: account, pickle: stored.pickle, storageKey: storageKey)
        }
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
    }

    /// to-device メッセージを処理する（Android の `CryptoStore.processToDevice`、TUI・React と同じ規則）。
    ///
    /// 1 件ごとに、変わった状態と処理済みの印を保存してから次に進む。ack の前に終了しても、同じメッセージを 2 回
    /// 復号しない。`keysQuery` は送信ユーザーと自分の `keys/query` の応答（端末鍵と署名を含む）で、解錠していれば、
    /// 自分と送信者のクロス署名と保存済みの信頼記録で送信端末を検証する。
    func processToDevice(_ rows: [ToDeviceRow], keysQuery: String) throws -> ToDeviceResult {
        try lock.withLock {
            var processed: [String] = [], roomKeys = 0, dropped = 0, newOlm = 0, verified = 0
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
                                  verifiedRoomKeys: verified)
        }
    }

    private enum Encrypted {
        case dropped
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
            guard case .object(let envelope) = try JSONDecoder().decode(JSONValue.self, from: Data(message.ciphertext.utf8)),
                  case .string(let senderKey) = envelope["sender_key"], case .string(let sessionID) = envelope["session_id"] else {
                throw ZooError.invalidResponse
            }
            try loadInbound(Self.inboundKey(SessionRef(channelId: message.channelId, senderKey: senderKey, sessionId: sessionID)))
            return try inbound.decrypt(message: message)
        }
    }

    // MARK: - 送信

    /// 解錠済みの鍵で配布先を判定し、TOFU の変更を保存する。
    func recipients(pages: [String]) throws -> Distribution {
        try lock.withLock {
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
    func encrypt(_ outbound: OutboundSession, message: OutgoingMessage) throws -> String {
        try lock.withLock {
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
            }
            return try outbound.encrypt(account: account, message: message)
        }
    }

    /// 暗号ストアを閉じる（同じ暗号ストアをまた開けるようにする）。
    func close() {
        lock.withLock {
            guard !closed else { return }
            closed = true
            _ = Self.openDirectories.withLock { $0.remove(directory.standardizedFileURL.path) }
        }
    }

    /// ログアウトで暗号ストアを消す。`state.json` を消し、保存鍵を Keychain から消す。
    func destroy(_ files: CryptoStoreFiles) throws {
        close()
        let stateURL = directory.appendingPathComponent(Self.stateFile)
        if FileManager.default.fileExists(atPath: stateURL.path) { try FileManager.default.removeItem(at: stateURL) }
        try files.keys.delete()
    }

    private func save() throws {
        try Self.writeAtomically(JSONEncoder().encode(state), to: directory.appendingPathComponent(Self.stateFile))
    }

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
        _ = try FileManager.default.replaceItemAt(url, withItemAt: temporary)
    }
}
