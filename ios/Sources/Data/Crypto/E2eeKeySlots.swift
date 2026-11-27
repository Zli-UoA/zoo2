import Foundation
import Observation
import ZooCrypto

/// 鍵スロット 1 件の表示用の行（`GET /api/e2ee/secret_storage` の `slots`）。
struct KeySlotRow: Identifiable, Equatable {
    let id: String
    /// `passkey` か `recovery`。
    let kind: String
    let label: String?
    let createdAt: String?
    let lastUsedAt: String?

    var title: String {
        if let label, !label.isEmpty { return label }
        return kind == "passkey" ? "パスキー" : "リカバリーキー"
    }
}

/// 解錠したあとの、鍵スロット（パスキーとリカバリーキー）の追加と削除（docs/e2ee.md の「鍵スロット」）。
/// 追加には、解錠した SSSS 鍵が要るので、そのつど、パスキーかリカバリーキーで SSSS を開け直してもらう。追加と削除には、
/// 直前の再認証が要る。最後の 1 つは消せない（解錠する手段がなくなるため）。
@Observable @MainActor
final class E2eeKeySlots {
    private(set) var slots: [KeySlotRow] = []

    @ObservationIgnored private let api: any E2eeRevocationAPI
    @ObservationIgnored private let userID: String

    init(api: any E2eeRevocationAPI, userID: String) {
        self.api = api
        self.userID = userID
    }

    func load() async throws {
        let storage = try await api.call("api/e2ee/secret_storage", method: "GET", query: [:], body: nil, reauth: nil, headers: [:])
        slots = (storage.object?["slots"]?.array ?? []).compactMap { value in
            guard let object = value.object, let id = object["id"]?.string, let kind = object["kind"]?.string else { return nil }
            return KeySlotRow(id: id, kind: kind, label: object["label"]?.string,
                              createdAt: object["created_at"]?.string, lastUsedAt: object["last_used_at"]?.string)
        }
    }

    /// SSSS を開け直す。
    func open(_ method: UnlockMethod) async throws -> UnlockedSecretStorage {
        try await e2eeOpenSecretStorage(api: api, userID: userID, method: method)
    }

    /// パスキーのスロットを足す（新しい `prf_salt`。利用者がパスキーを選ぶ）。
    func addPasskey(unlocked: UnlockedSecretStorage, client: any PasskeyClient, rpID: String, label: String?,
                    reauth: () async throws -> String) async throws {
        let salt = newPrfSalt()
        let result = try await client.prfForNewSlot(rpID: rpID, salt: salt)
        let slot = try newPasskeyKeySlot(userId: userID, ssssKey: unlocked.key(), credentialId: result.credentialID,
                                         prfSalt: salt, prfOutput: result.prfOutput)
        try await post(slot, label: label, reauth: reauth)
    }

    struct PreparedRecovery: Equatable {
        let display: String
        fileprivate let slot: String
    }

    /// 控えたことを確認するまで、既存のリカバリーキーを使えるままにする。
    func prepareRecovery(unlocked: UnlockedSecretStorage) throws -> PreparedRecovery {
        let recovery = RecoveryKey.generate()
        let slot = try newRecoveryKeySlot(userId: userID, ssssKey: unlocked.key(), recoveryKey: recovery)
        return PreparedRecovery(display: recovery.toDisplay(), slot: slot)
    }

    /// サーバーが同じ SSSS 鍵のリカバリースロットを置き換える。成功後の一覧取得は別に行う。
    func saveRecovery(_ prepared: PreparedRecovery, reauth: () async throws -> String) async throws {
        try await post(prepared.slot, label: nil, reauth: reauth)
    }

    /// スロットを消す。最後の 1 つは消せない。
    func remove(_ slot: KeySlotRow, reauth: () async throws -> String) async throws {
        guard slots.count > 1 else { throw KeySlotError.lastSlot }
        try await withReauth(reauth) { token in
            guard let encoded = slot.id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed.subtracting(CharacterSet(charactersIn: "/"))) else {
                throw ZooError.invalidResponse
            }
            _ = try await self.api.call("api/e2ee/secret_storage/slots/\(encoded)", method: "DELETE", query: [:], body: nil, reauth: token, headers: [:])
        }
    }

    private func post(_ slot: String, label: String?, reauth: () async throws -> String) async throws {
        guard case .object(var body) = try JSONDecoder().decode(JSONValue.self, from: Data(slot.utf8)) else { throw ZooError.invalidResponse }
        // サーバーの `label` は省略できるが `null` は受け付けないので、ないときは送らない。
        body["label"] = nil
        if let label, !label.isEmpty { body["label"] = .string(label) }
        try await withReauth(reauth) { token in
            _ = try await self.api.call("api/e2ee/secret_storage/slots", method: "POST", query: [:], body: body, reauth: token, headers: [:])
        }
    }

    /// 直前の再認証つきで行う。足りなければ（403 `E2EE_REAUTH_REQUIRED`）取り直して、もう一度だけ行う。
    private func withReauth(_ reauth: () async throws -> String, _ operation: (String) async throws -> Void) async throws {
        var attempts = 0
        while true {
            let token = try await reauth()
            do { try await operation(token); return }
            catch let failure as E2eeAPIError where failure.isReauthRequired && attempts < 1 { attempts += 1 }
        }
    }
}

enum KeySlotError: Error, Equatable {
    /// 最後の 1 つは消せない。
    case lastSlot
}
