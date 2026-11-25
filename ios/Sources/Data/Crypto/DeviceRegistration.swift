import Foundation
import UIKit

/// 端末の登録に使う API（docs/e2ee.md の「端末とワンタイム鍵」）。
@MainActor
protocol E2eeDeviceAPI {
    /// `POST /api/e2ee/devices`。失敗は [E2eeAPIError]。
    func registerDevice(_ body: [String: JSONValue]) async throws
    /// `GET /api/e2ee/devices`。自分の端末（削除済みを除く）の `device_id`。
    func deviceIDs() async throws -> [String]
    /// `DELETE /api/e2ee/devices/{device_id}`。
    func deleteDevice(_ deviceID: String) async throws
}

/// API の失敗。`code` はサーバーの `E2EE_*`（ない応答は `nil`）。
struct E2eeAPIError: Error, Equatable {
    let status: Int
    let code: String?
    /// 409 `E2EE_STALE_EPOCH` の `current_epoch`。
    var currentEpoch: UInt64? = nil

    /// 登録の 409 `E2EE_DEVICE_EXISTS`（削除済みの `device_id`）。`code` のない 409 は含めない。
    var isDeletedDeviceID: Bool { status == 409 && code == "E2EE_DEVICE_EXISTS" }
    /// 鍵のアップロードや ack の 404 `E2EE_DEVICE_NOT_FOUND`（消された端末の候補）。`code` のない 404 は含めない。
    var isDeviceNotFound: Bool { status == 404 && code == "E2EE_DEVICE_NOT_FOUND" }
    /// 直前の再認証が要る（403 `E2EE_REAUTH_REQUIRED`）。
    var isReauthRequired: Bool { status == 403 && code == "E2EE_REAUTH_REQUIRED" }
    /// 別の端末が先に SSSS を書いた（409 `E2EE_SSSS_CONFLICT`）。
    var isSsssConflict: Bool { status == 409 && code == "E2EE_SSSS_CONFLICT" }
}

extension APIClient: E2eeDeviceAPI {
    func registerDevice(_ body: [String: JSONValue]) async throws {
        _ = try await e2ee("api/e2ee/devices", method: "POST", body: body)
    }

    func deviceIDs() async throws -> [String] {
        struct Device: Decodable { let device_id: String }
        let data = try await e2ee("api/e2ee/devices", method: "GET")
        let decoder = JSONDecoder()
        if let devices = try? decoder.decode([Device].self, from: data) { return devices.map(\.device_id) }
        return try decoder.decode(Envelope<[Device]>.self, from: data).data.map(\.device_id)
    }

    func deleteDevice(_ deviceID: String) async throws {
        guard UUID(uuidString: deviceID) != nil else { throw ZooError.invalidResponse }
        _ = try await e2ee("api/e2ee/devices/\(deviceID.lowercased())", method: "DELETE")
    }

    /// E2EE の API は `code` を見て判断するので、`HTTPTransport.check` ではなく、ここで `code` を読む。
    func e2ee(_ path: String, method: String, body: [String: JSONValue]? = nil, reauth: String? = nil) async throws -> Data {
        var request = try await request(path, method: method, body: body)
        // 直前の再認証で受け取った ID トークン。
        if let reauth { request.setValue(reauth, forHTTPHeaderField: "X-Zoo-Reauth") }
        let (data, response) = try await transport.send(request)
        if response.statusCode == 401 { throw ZooError.expiredSession }
        guard (200..<300).contains(response.statusCode) else {
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            throw E2eeAPIError(status: response.statusCode, code: json?["code"] as? String,
                               currentEpoch: (json?["current_epoch"] as? NSNumber)?.uint64Value)
        }
        return data
    }
}

/// 端末の登録。応答をなくしても同じ鍵で送り直せる（サーバーは同じ `device_keys` なら成功にする）。
@MainActor
struct DeviceRegistration {
    let api: any E2eeDeviceAPI
    let files: CryptoStoreFiles
    let userID: String
    var displayName = Self.defaultDisplayName()

    /// 設定の「端末」に出す名前。iPad は `iPad（iOS）`、それ以外は `iPhone（iOS）`。
    static func defaultDisplayName(idiom: UIUserInterfaceIdiom = UIDevice.current.userInterfaceIdiom) -> String {
        idiom == .pad ? "iPad（iOS）" : "iPhone（iOS）"
    }

    /// 暗号ストアを開き、登録していなければ登録する。
    ///
    /// - 保存鍵を失って作り直したときは、前の端末をサーバーから消す（消せなければ次回に持ち越さず、失敗として投げる）。
    /// - 登録が 409 `E2EE_DEVICE_EXISTS` のときは、`GET /api/e2ee/devices` の一覧に今の端末がないと確かめられたときだけ、
    ///   別の端末から削除されたとみなす。暗号ストアは消さず、端末だけを作り直して登録し直す。
    ///   一覧に今の端末がある、一覧を取れないときは、ふつうの失敗として投げる。
    func ensureRegistered() async throws -> CryptoStore {
        guard let opened = try CryptoStore.open(files, userID: userID) else { throw CocoaError(.fileNoSuchFile) }
        let store = opened.store
        do {
            // 保存鍵を失って作り直した前の端末を、サーバーから消す。消せるまで、記録に残す。
            for replaced in store.pendingDeviceDeletions {
                try await deleteReplaced(replaced)
                try store.clearPendingDeletion(replaced)
            }
            guard !store.isRegistered else { return store }
            try await register(store)
            return store
        } catch {
            store.close()
            throw error
        }
    }

    func register(_ store: CryptoStore, recreated: Bool = false) async throws {
        var body = try store.registrationRequest()
        body["display_name"] = .string(displayName)
        do {
            try await api.registerDevice(body)
        } catch let failure as E2eeAPIError where failure.isDeletedDeviceID {
            // 自分で消した端末の ID は再利用できない。一覧に今の端末がなければ、新しい端末で登録し直す。
            guard !recreated, !(try await api.deviceIDs()).contains(store.deviceID) else { throw failure }
            try store.replaceDevice()
            try await register(store, recreated: true)
            return
        }
        try store.markRegistered()
    }

    private func deleteReplaced(_ deviceID: String) async throws {
        do { try await api.deleteDevice(deviceID) } catch let failure as E2eeAPIError where failure.status == 404 {
            // すでに消えている。
        }
    }
}
