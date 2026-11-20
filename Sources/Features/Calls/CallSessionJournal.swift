import Foundation
import Security

struct CallSessionRecord: Codable, Equatable, Sendable {
    let channelID: String
    let sessionID: String
}

@MainActor
protocol CallSessionJournal {
    func load() throws -> [CallSessionRecord]
    func save(_ records: [CallSessionRecord]) throws
}

extension CallSessionJournal {
    func record(_ session: CallSessionRecord) throws {
        var records = try load()
        if !records.contains(session) { records.append(session) }
        try save(records)
    }

    func remove(_ session: CallSessionRecord) throws {
        // 退出応答を待つ間に始まった別の通話の記録は残す。
        try save(load().filter { $0 != session })
    }
}

@MainActor
struct KeychainCallSessionJournal: CallSessionJournal {
    let account: String
    var service = "works.zli.zoo.call-sessions"

    private var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service, kSecAttrAccount as String: account]
    }

    func load() throws -> [CallSessionRecord] {
        var request = query
        request[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess, let data = result as? Data else {
            throw ZooError.auth("前回の通話情報を読み取れませんでした。端末のロックを解除して再度お試しください。")
        }
        return try JSONDecoder().decode([CallSessionRecord].self, from: data)
    }

    func save(_ records: [CallSessionRecord]) throws {
        if records.isEmpty {
            let status = SecItemDelete(query as CFDictionary)
            guard status == errSecSuccess || status == errSecItemNotFound else {
                throw ZooError.auth("終了した通話情報を削除できませんでした。")
            }
            return
        }
        let attributes: [String: Any] = [
            kSecValueData as String: try JSONEncoder().encode(records),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        ]
        var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            status = SecItemAdd(query.merging(attributes) { _, new in new } as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw ZooError.auth("通話情報を安全に保存できませんでした。") }
    }
}
