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
    var revocationPending: Bool?
    var kind: PushKind? = nil
    var id: String { Self.digest((kind == .voip ? "voip:" : "") + "\(environment):\(token)") }
    static func digest(_ value: String) -> String { SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined() }
}

enum PushKind: String, Codable { case alert, voip }

@MainActor
protocol PushReceiptStore {
    func load() throws -> PushReceipt?
    func save(_ receipt: PushReceipt?) throws
}

@MainActor
struct KeychainPushReceiptStore: PushReceiptStore {
    var service = "works.zli.zoo.push"
    private var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service, kSecAttrAccount as String: "registration"]
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
    private let sessionID: UUID?
    init(api: APIClient?, transport: HTTPTransport) {
        self.api = api
        self.transport = transport
        sessionID = api?.auth.sessionID
    }
    var server: String { api?.settings.server ?? "" }
    func subscribe(_ receipt: PushReceipt) async throws {
        guard let api else { throw ZooError.expiredSession }
        guard api.auth.sessionID == sessionID else { throw CancellationError() }
        struct Configuration: Decodable, Sendable { let sandbox: Bool; let production: Bool; let voip: Bool? }
        let config: Configuration = try await api.get("api/push/apns/config")
        guard api.auth.sessionID == sessionID else { throw CancellationError() }
        guard receipt.environment == "sandbox" ? config.sandbox : config.production else { throw ZooError.auth("サーバーのプッシュ通知設定が完了していません。") }
        guard receipt.kind != .voip || config.voip == true else {
            throw ZooError.auth("サーバーの通話通知対応を待っています。")
        }
        let request = try await api.request("api/push/apns/subscriptions", method: "POST", body: [
            "device_token": .string(receipt.token), "environment": .string(receipt.environment),
            "kind": .string((receipt.kind ?? .alert).rawValue),
            "revocation_secret": .string(receipt.secret), "created_at": .number(Double(receipt.createdAt))])
        guard api.auth.sessionID == sessionID else { throw CancellationError() }
        let (data, response) = try await transport.send(request)
        try HTTPTransport.check(data, response)
        struct Response: Decodable { let id: String }
        guard try JSONDecoder().decode(Response.self, from: data).id == receipt.id else { throw ZooError.invalidResponse }
    }

    func sendTest(to receipt: PushReceipt, account: String, id: UUID) async throws {
        guard let api, receipt.server == api.settings.server, receipt.account == account,
              receipt.revocationPending != true else { throw ZooError.auth("この端末の通知登録を確認してください。") }
        let data = try await api.data("api/push/apns/test", method: "POST", body: [
            "id": .string(receipt.id), "revocation_secret": .string(receipt.secret), "test_id": .string(id.uuidString)])
        struct Response: Decodable { let ok: Bool }
        guard try JSONDecoder().decode(Response.self, from: data).ok else { throw ZooError.invalidResponse }
    }
    func revoke(_ receipt: PushReceipt) async throws {
        // 登録解除には元のサーバーと解除専用の秘密情報だけを使い、新しいアカウントの Bearer トークンは使わない。
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
    private let kind: PushKind
    private var tail: Task<Void, Never>?
    init(store: any PushReceiptStore = KeychainPushReceiptStore(), kind: PushKind = .alert) {
        self.store = store; self.kind = kind
    }

    // 中断を挟む処理も直列化し、進行中の登録が終わってからログアウトに伴う解除を行う。
    @discardableResult
    func update(backend: any PushBackend, account: String?, token: String?, environment: String,
                completion: @escaping @MainActor (Result<PushReceipt?, Error>) -> Void = { _ in }) -> Task<Void, Never> {
        let previous = tail
        let task = Task {
            await previous?.value
            do {
                let receipt = try await self.reconcile(backend: backend, account: account, token: token, environment: environment)
                completion(.success(receipt))
            } catch { completion(.failure(error)) }
        }
        tail = task
        return task
    }

    private func reconcile(backend: any PushBackend, account: String?, token: String?, environment: String) async throws -> PushReceipt? {
        var receipt = try store.load()
        if var old = receipt, old.revocationPending == true || old.server != backend.server || old.account != account ||
            (token != nil && old.token != token) || old.environment != environment {
            if old.revocationPending != true {
                // 解除応答が失われたときも、同じアカウントで失効済みの解除情報を再利用しない。
                old.revocationPending = true
                try store.save(old)
            }
            try await backend.revoke(old)
            try store.save(nil)
            receipt = nil
        }
        guard let account, let token else { return nil }
        let now = Int64(Date().timeIntervalSince1970)
        if var existing = receipt {
            if existing.createdAt < now - 540 || existing.createdAt > now + 30 {
                existing.createdAt = now
                try store.save(existing)
                receipt = existing
            }
        } else {
            var bytes = [UInt8](repeating: 0, count: 32)
            guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { throw ZooError.invalidResponse }
            receipt = PushReceipt(server: backend.server, account: account, token: token, environment: environment,
                                  secret: bytes.map { String(format: "%02x", $0) }.joined(), kind: kind)
            // HTTP 応答が失われても解除や再試行ができるよう、送信前に登録情報を永続化する。
            try store.save(receipt)
        }
        if let receipt { try await backend.subscribe(receipt) }
        return receipt
    }
}
