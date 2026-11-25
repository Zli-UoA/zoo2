import Foundation

/// `GET /api/e2ee/devices` の 1 件（自分の端末だけ。端末名と最後に使った日時は本人にだけ返る）。
struct OwnDevice: Decodable, Identifiable, Equatable, Sendable {
    let device_id: String
    let display_name: String?
    let created_at: String?
    let last_seen_at: String?

    var id: String { device_id }
    var name: String { display_name.flatMap { $0.isEmpty ? nil : $0 } ?? "名前のない端末" }

    /// 一覧の並び：登録が新しい順（日時が読めないものは後ろ）。
    static func ordered(_ devices: [OwnDevice]) -> [OwnDevice] {
        devices.sorted { ($0.created_at ?? "") > ($1.created_at ?? "") }
    }
}

/// 端末一覧の API（docs/e2ee.md の「端末一覧と失効」のうち、一覧・名前の変更・削除）。
@MainActor
protocol E2eeOwnDevicesAPI {
    func ownDevices() async throws -> [OwnDevice]
    func renameDevice(_ deviceID: String, to name: String) async throws
    func removeDevice(_ deviceID: String) async throws
}

extension APIClient: E2eeOwnDevicesAPI {
    func ownDevices() async throws -> [OwnDevice] {
        let data = try await e2ee("api/e2ee/devices", method: "GET")
        let decoder = JSONDecoder()
        if let devices = try? decoder.decode([OwnDevice].self, from: data) { return OwnDevice.ordered(devices) }
        return OwnDevice.ordered(try decoder.decode(Envelope<[OwnDevice]>.self, from: data).data)
    }

    func renameDevice(_ deviceID: String, to name: String) async throws {
        guard UUID(uuidString: deviceID) != nil else { throw ZooError.invalidResponse }
        _ = try await e2ee("api/e2ee/devices/\(deviceID.lowercased())", method: "PATCH", body: ["display_name": .string(name)])
    }

    func removeDevice(_ deviceID: String) async throws {
        try await deleteDevice(deviceID)
    }
}
