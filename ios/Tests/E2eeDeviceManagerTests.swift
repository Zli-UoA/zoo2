import Foundation
import Testing
@testable import Zoo

@MainActor
private final class FakeDeviceServer: E2eeDeviceAPI, E2eeKeysAPI {
    var registered: [[String: JSONValue]] = []
    var uploads: [(oneTime: Int, fallback: Int)] = []
    var acked: [[String]] = []
    var counts = KeyCounts(oneTimeKeys: 0, hasUnusedFallbackKey: false)
    var listed: [String] = []
    var failUploadNotFound = 0
    var queriedUsers: [[String]] = []

    func registerDevice(_ body: [String: JSONValue]) async throws { registered.append(body) }
    func deviceIDs() async throws -> [String] { listed }
    func deleteDevice(_ deviceID: String) async throws {}

    func uploadKeys(deviceID: String, oneTimeKeys: [String: JSONValue], fallbackKeys: [String: JSONValue]) async throws -> KeyCounts {
        if failUploadNotFound > 0 { failUploadNotFound -= 1; throw E2eeAPIError(status: 404, code: "E2EE_DEVICE_NOT_FOUND") }
        uploads.append((oneTimeKeys.count, fallbackKeys.count))
        if !oneTimeKeys.isEmpty { counts = KeyCounts(oneTimeKeys: counts.oneTimeKeys + oneTimeKeys.count, hasUnusedFallbackKey: true) }
        return counts
    }

    func queryKeys(userIDs: [String]) async throws -> String {
        queriedUsers.append(userIDs)
        return #"{"device_keys":{},"signatures":[],"master_keys":{},"self_signing_keys":{},"user_signing_keys":{},"tofu_pins":{},"user_ids":[]}"#
    }

    func ackToDevice(deviceID: String, ids: [String]) async throws { acked.append(ids) }
}

@MainActor
private final class ToDeviceFeed {
    var deliver: (@MainActor ([CryptoStore.ToDeviceRow]) -> Void)?
    var starts = 0
    var failNext = false

    func observe(_ deviceID: String, _ receive: @escaping @MainActor ([CryptoStore.ToDeviceRow]) -> Void) async throws {
        starts += 1
        if failNext { failNext = false; throw ZooError.invalidResponse }
        deliver = receive
        receive([])
        try await Task.sleep(for: .seconds(60))
    }
}

@MainActor
private func eventually(_ message: String = "", _ condition: () -> Bool) async -> Bool {
    for _ in 0..<400 {
        if condition() { return true }
        try? await Task.sleep(for: .milliseconds(10))
    }
    return false
}

@MainActor
struct E2eeDeviceManagerTests {
    private func make(_ server: FakeDeviceServer, _ feed: ToDeviceFeed) -> (E2eeDeviceManager, URL) {
        let (files, _, directory) = makeFiles()
        let manager = E2eeDeviceManager(api: server, files: files, userID: user,
                                        observeToDevice: { try await feed.observe($0, $1) },
                                        firstRetryDelay: .milliseconds(20), maxRetryDelay: .milliseconds(80))
        return (manager, directory)
    }

    @Test func 登録しワンタイム鍵を補充してto_deviceの行をackする() async throws {
        let server = FakeDeviceServer(), feed = ToDeviceFeed()
        let (manager, directory) = make(server, feed)
        defer { try? FileManager.default.removeItem(at: directory) }
        manager.start()
        #expect(await eventually { manager.readyStore != nil && feed.deliver != nil })
        #expect(server.registered.count == 1)
        // 数を読む空のアップロードのあとに、足りない分を補充する（最大数と、フォールバック鍵 1 つ）。
        #expect(server.uploads.first?.oneTime == 0)
        #expect(server.uploads.contains { $0.oneTime > 0 && $0.fallback == 1 })
        // 復号できない行は、捨てて ack する。
        feed.deliver?([.init(id: "5", senderUserID: user, type: "m.room.encrypted", content: #"{"algorithm":"x"}"#),
                       .init(id: "6", senderUserID: user, type: "m.dummy", content: "{}")])
        #expect(await eventually { !server.acked.isEmpty })
        #expect(server.acked.first == ["5", "6"])
        #expect(server.queriedUsers.first == [user])
        // ack より先に同じ一覧が届いても、処理し直さない。
        feed.deliver?([.init(id: "5", senderUserID: user, type: "m.room.encrypted", content: "{}")])
        try await Task.sleep(for: .milliseconds(100))
        #expect(server.acked.count == 1)
        await manager.stop()
        #expect(manager.readyStore == nil)
    }

    @Test func 購読が切れたら間隔をあけて初めからやり直す() async throws {
        let server = FakeDeviceServer(), feed = ToDeviceFeed()
        feed.failNext = true
        let (manager, directory) = make(server, feed)
        defer { try? FileManager.default.removeItem(at: directory) }
        manager.start()
        #expect(await eventually { manager.readyStore != nil && feed.starts >= 2 })
        // 登録は 1 回だけ（開き直しても、登録済みの端末を登録し直さない）。
        #expect(server.registered.count == 1)
        await manager.stop()
    }

    @Test func 別の端末から消された端末は一覧にないときだけ作り直して登録し直す() async throws {
        let server = FakeDeviceServer(), feed = ToDeviceFeed()
        server.failUploadNotFound = 1
        let (manager, directory) = make(server, feed)
        defer { try? FileManager.default.removeItem(at: directory) }
        manager.start()
        #expect(await eventually { manager.readyStore != nil && server.registered.count == 2 })
        guard case .object(let first)? = server.registered.first?["device_keys"], case .object(let second)? = server.registered.last?["device_keys"] else {
            Issue.record("device_keys がない"); return
        }
        #expect(first["device_id"] != second["device_id"])
        await manager.stop()
    }

    @Test func 行はIDの桁数と文字列の順に並べ文字列のjsonbも読む() throws {
        let rows = [ToDeviceShapeRow(id: "10", sender_user_id: "u", type: "t", content: .string(#"{"a":1}"#)),
                    ToDeviceShapeRow(id: "9", sender_user_id: "u", type: "t", content: .object(["b": .number(2)])),
                    ToDeviceShapeRow(id: "100", sender_user_id: "u", type: "t", content: .string("not json"))]
        let ordered = ToDeviceShapeRow.ordered(rows)
        #expect(ordered.map(\.id) == ["9", "10", "100"])
        #expect(ordered[1].content == #"{"a":1}"#)
        #expect(ordered[2].content == "null")
    }

    @Test func ワンタイム鍵の数は応答から読む() {
        let counts = KeyCounts(.object(["one_time_key_counts": .object(["signed_curve25519": .number(50)]),
                                        "unused_fallback_key_types": .array([.string("signed_curve25519")])]))
        #expect(counts == KeyCounts(oneTimeKeys: 50, hasUnusedFallbackKey: true))
        #expect(KeyCounts(.object([:])) == KeyCounts())
    }
}
