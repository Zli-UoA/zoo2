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

/// 同じ暗号ストアを、ほかの場所が開いている。少し待って試し直す。
struct CryptoStoreBusyError: Error, Equatable {}

/// `state.json` の中身。`account` は `DeviceAccount.pickle`（保存鍵で暗号化済み）。
/// `user_id`、`device_id`、登録済みかは平文で、秘密ではない（Android・TUI の `state.json` と同じ形）。
private struct StoreState: Codable {
    var version: Int
    var user_id: String
    var device_id: String
    var registered: Bool
    var account: String
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
    private var state: StoreState
    private var closed = false
    private let lock = NSLock()

    var userID: String { state.user_id }
    var deviceID: String { state.device_id }
    var isRegistered: Bool { lock.withLock { state.registered } }

    private init(directory: URL, storageKey: Data, account: DeviceAccount, state: StoreState) {
        self.directory = directory
        self.storageKey = storageKey
        self.account = account
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
                return Opened(store: CryptoStore(directory: files.directory, storageKey: key, account: account, state: stored),
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
        let state = StoreState(version: stateVersion, user_id: userID, device_id: account.deviceId(),
                               registered: false, account: try account.pickle(storageKey: key))
        let store = CryptoStore(directory: files.directory, storageKey: key, account: account, state: state)
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
            nextState.account = try next.pickle(storageKey: storageKey)
            try Self.writeAtomically(JSONEncoder().encode(nextState), to: directory.appendingPathComponent(Self.stateFile))
            // 保存できてから、メモリーの状態を進める。
            account = next
            state = nextState
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
