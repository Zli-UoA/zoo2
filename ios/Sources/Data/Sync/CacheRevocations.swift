import CryptoKit
import Foundation
import Security

protocol CacheRevocationStorage: Sendable {
    func contains(_ scope: String) throws -> Bool
    func revoke(_ scope: String) throws
    func allow(_ scope: String) throws
}

// DB と許可ファイルを消せなくても、別の保存先で読出しを止める。
struct KeychainCacheRevocations: CacheRevocationStorage {
    let namespace: String

    private func query(_ scope: String) -> [String: Any] {
        let key = SHA256.hash(data: Data((namespace + "|" + scope).utf8))
            .map { String(format: "%02x", $0) }.joined()
        return [kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: "works.zli.zoo.cache-revocations",
                kSecAttrAccount as String: key]
    }

    private func check(_ status: OSStatus) throws {
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
        }
    }

    func contains(_ scope: String) throws -> Bool {
        let status = SecItemCopyMatching(query(scope) as CFDictionary, nil)
        try check(status)
        return status == errSecSuccess
    }

    func revoke(_ scope: String) throws {
        let attributes: [String: Any] = [kSecValueData as String: Data([1]),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        let request = query(scope)
        var status = SecItemUpdate(request as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            status = SecItemAdd(request.merging(attributes) { _, new in new } as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
    }

    func allow(_ scope: String) throws { try check(SecItemDelete(query(scope) as CFDictionary)) }
}
