import CryptoKit
import Foundation
import Security

struct PushReceipt: Codable, Equatable {
    let server: String
    let account: String
    let token: String
    let environment: String
    let secret: String
    var createdAt: Int64 = Int64(Date().timeIntervalSince1970)
    var id: String { Self.digest("\(environment):\(token)") }
    static func digest(_ value: String) -> String { SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined() }
}

@MainActor
protocol PushReceiptStore {
    func load() throws -> PushReceipt?
    func save(_ receipt: PushReceipt?) throws
}

@MainActor
struct KeychainPushReceiptStore: PushReceiptStore {
    private var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "works.zli.zoo.push", kSecAttrAccount as String: "registration"]
    }
    func load() throws -> PushReceipt? {
        var request = query
        request[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw ZooError.auth("通知設定を読み取れませんでした。") }
        return try JSONDecoder().decode(PushReceipt.self, from: data)
    }
    func save(_ receipt: PushReceipt?) throws {
        guard let receipt else {
            let status = SecItemDelete(query as CFDictionary)
            guard status == errSecSuccess || status == errSecItemNotFound else { throw ZooError.auth("通知設定を削除できませんでした。") }
            return
        }
        let attributes: [String: Any] = [kSecValueData as String: try JSONEncoder().encode(receipt),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var request = query
            request.merge(attributes) { _, new in new }
            status = SecItemAdd(request as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw ZooError.auth("通知設定を安全に保存できませんでした。") }
    }
}

@MainActor
protocol PushBackend {
    var server: String { get }
    func subscribe(_ receipt: PushReceipt) async throws
    func revoke(_ receipt: PushReceipt) async throws
}

@MainActor
struct LivePushBackend: PushBackend {
    let api: APIClient?
    let transport: HTTPTransport
    var server: String { api?.settings.server ?? "" }
    func subscribe(_ receipt: PushReceipt) async throws {
        guard let api else { throw ZooError.expiredSession }
        struct Configuration: Decodable, Sendable { let sandbox: Bool; let production: Bool }
        let config: Configuration = try await api.get("api/push/apns/config")
        guard receipt.environment == "sandbox" ? config.sandbox : config.production else { throw ZooError.auth("サーバーのプッシュ通知設定が完了していません。") }
        let data = try await api.data("api/push/apns/subscriptions", method: "POST", body: [
            "device_token": .string(receipt.token), "environment": .string(receipt.environment),
            "revocation_secret": .string(receipt.secret), "created_at": .number(Double(receipt.createdAt))])
        struct Response: Decodable { let id: String }
        guard try JSONDecoder().decode(Response.self, from: data).id == receipt.id else { throw ZooError.invalidResponse }
    }
    func revoke(_ receipt: PushReceipt) async throws {
        // Cleanup uses only the original server and its narrowly scoped capability, never a new account's bearer token.
        var request = URLRequest(url: try ConnectionSettings.secureURL(receipt.server).appendingPathComponent("api/push/apns/revoke"))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(["id": receipt.id, "revocation_secret": receipt.secret])
        let (data, response) = try await transport.send(request)
        try HTTPTransport.check(data, response)
        struct Response: Decodable { let ok: Bool }
        guard try JSONDecoder().decode(Response.self, from: data).ok else { throw ZooError.invalidResponse }
    }
}

@MainActor
final class PushRegistration {
    private let store: any PushReceiptStore
    private var tail: Task<Void, Never>?
    init(store: any PushReceiptStore = KeychainPushReceiptStore()) { self.store = store }

    // Operations are serialized across suspension points so logout always follows any registration in flight.
    @discardableResult
    func update(backend: any PushBackend, account: String?, token: String?, environment: String,
                completion: @escaping @MainActor (Error?) -> Void = { _ in }) -> Task<Void, Never> {
        let previous = tail
        let task = Task {
            await previous?.value
            do {
                try await self.reconcile(backend: backend, account: account, token: token, environment: environment)
                completion(nil)
            } catch { completion(error) }
        }
        tail = task
        return task
    }

    private func reconcile(backend: any PushBackend, account: String?, token: String?, environment: String) async throws {
        var receipt = try store.load()
        if let old = receipt, old.server != backend.server || old.account != account ||
            (token != nil && old.token != token) || old.environment != environment ||
            Date().timeIntervalSince1970 - Double(old.createdAt) > 540 ||
            Double(old.createdAt) - Date().timeIntervalSince1970 > 30 {
            try await backend.revoke(old)
            try store.save(nil)
            receipt = nil
        }
        guard let account, let token else { return }
        if receipt == nil {
            var bytes = [UInt8](repeating: 0, count: 32)
            guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { throw ZooError.invalidResponse }
            receipt = PushReceipt(server: backend.server, account: account, token: token, environment: environment,
                                  secret: bytes.map { String(format: "%02x", $0) }.joined())
            // Persist before sending: a lost HTTP response must still leave enough information to revoke or retry.
            try store.save(receipt)
        }
        if let receipt { try await backend.subscribe(receipt) }
    }
}
