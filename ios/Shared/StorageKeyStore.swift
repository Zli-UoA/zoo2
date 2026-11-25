import Foundation
import Security

/// 暗号ストアの保存鍵（32 バイト）の置き場。Keychain は端末から出せず、アプリを消しても残ることがあるので、
/// 保存鍵だけを Keychain に置き、暗号化した状態は別のファイルに置く（docs/e2ee.md の「暗号ストア」）。
protocol StorageKeyBacking: Sendable {
    func load() throws -> Data?
    func save(_ key: Data) throws
    func delete() throws
}

/// Keychain の汎用パスワード項目。`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` なので、
/// 端末の初回解除後はバックグラウンドの通知拡張からも読め、別の端末へは移らない。
/// アクセスグループは通知の鍵と同じ共有グループ（`ZooPreviewKeychainGroup`）を使い、拡張と共有する。
struct KeychainStorageKey: StorageKeyBacking {
    let account: String
    var group: String? = Bundle.main.object(forInfoDictionaryKey: "ZooPreviewKeychainGroup") as? String

    static let service = "works.zli.zoo.e2ee.storage-key"

    private func query() throws -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.service,
            kSecAttrAccount as String: account
        ]
        // 共有グループを使えないとき（単体テストなど）はアプリ自身のグループに置く。
        if let group, !group.isEmpty, !group.contains("$(") { query[kSecAttrAccessGroup as String] = group }
        return query
    }

    private func error(_ status: OSStatus) -> NSError { NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }

    func load() throws -> Data? {
        var request = try query()
        request[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw error(status) }
        guard let data = result as? Data else { throw error(errSecDecode) }
        return data
    }

    func save(_ key: Data) throws {
        let query = try query()
        let attributes: [String: Any] = [
            kSecValueData as String: key,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        ]
        var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            status = SecItemAdd(query.merging(attributes) { _, new in new } as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw error(status) }
    }

    func delete() throws {
        let status = SecItemDelete(try query() as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw error(status) }
    }
}

enum StorageKeyError: Error, Equatable {
    /// Keychain の保存鍵が 32 バイトでない。壊れているので、暗号ストアを作り直す。
    case malformed
}

/// 保存鍵を読む、なければ作る。Keychain の一時的な失敗は、そのまま投げる
/// （暗号ストアを作り直すと、Olm セッションや、まだバックアップしていない受信用セッションを失うため）。
struct StorageKeyStore: Sendable {
    static let size = 32
    let backing: any StorageKeyBacking

    /// 返した値は、使い終わったら呼び出し側で捨てる。
    func loadOrCreate() throws -> Data {
        if let existing = try loadExisting() { return existing }
        var key = Data(count: Self.size)
        let status = key.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, Self.size, $0.baseAddress!) }
        guard status == errSecSuccess else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
        try backing.save(key)
        return key
    }

    /// なければ `nil`（作らない）。暗号ストアの状態はあるのに保存鍵がないときに、新しい鍵を作って取り違えない。
    func loadExisting() throws -> Data? {
        guard let key = try backing.load() else { return nil }
        guard key.count == Self.size else { throw StorageKeyError.malformed }
        return key
    }

    func delete() throws { try backing.delete() }
}
