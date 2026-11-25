import Foundation
import Testing
@testable import Zoo

private final class MemoryKey: StorageKeyBacking, @unchecked Sendable {
    private let lock = NSLock()
    private var value: Data?
    func load() throws -> Data? { lock.withLock { value } }
    func save(_ key: Data) throws { lock.withLock { value = key } }
    func delete() throws { lock.withLock { value = nil } }
}

private let user = "11111111-1111-4111-8111-111111111111"

private func makeFiles(_ key: MemoryKey = MemoryKey()) -> (CryptoStoreFiles, MemoryKey, URL) {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    return (CryptoStoreFiles(directory: directory, keys: StorageKeyStore(backing: key)), key, directory)
}

struct CryptoStoreTests {
    @Test func 保存鍵は作ったあと同じ値を返し32バイトである() throws {
        let store = StorageKeyStore(backing: MemoryKey())
        let first = try store.loadOrCreate()
        #expect(first.count == 32)
        #expect(try store.loadOrCreate() == first)
        try store.delete()
        #expect(try store.loadExisting() == nil)
    }

    @Test func 壊れた保存鍵は作り直さずにエラーにする() throws {
        let key = MemoryKey()
        try key.save(Data(count: 5))
        #expect(throws: StorageKeyError.malformed) { try StorageKeyStore(backing: key).loadOrCreate() }
    }

    @Test func 開き直しても同じ端末で登録の状態が残る() throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = try #require(try CryptoStore.open(files, userID: user)).store
        let deviceID = first.deviceID
        #expect(!first.isRegistered)
        try first.markRegistered()
        first.close()
        let second = try #require(try CryptoStore.open(files, userID: user))
        #expect(second.store.deviceID == deviceID)
        #expect(second.store.isRegistered)
        #expect(second.replacedDeviceID == nil)
        second.store.close()
    }

    @Test func 同じ暗号ストアを同時に2つは開けない() throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = try #require(try CryptoStore.open(files, userID: user)).store
        #expect(throws: CryptoStoreBusyError.self) { try CryptoStore.open(files, userID: user) }
        first.close()
        try #require(try CryptoStore.open(files, userID: user)).store.close()
    }

    @Test func 保存鍵を失うと新しい端末を作り前の端末のIDを返す() throws {
        let (files, key, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = try #require(try CryptoStore.open(files, userID: user)).store
        let oldID = first.deviceID
        first.close()
        try key.delete()
        let second = try #require(try CryptoStore.open(files, userID: user))
        #expect(second.replacedDeviceID == oldID)
        #expect(second.store.deviceID != oldID)
        second.store.close()
    }

    @Test func 別のユーザーの暗号ストアは開かない() throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        try #require(try CryptoStore.open(files, userID: user)).store.close()
        #expect(throws: (any Error).self) { try CryptoStore.open(files, userID: "33333333-3333-4333-8333-333333333333") }
    }

    @Test func 登録の本文に端末鍵とワンタイム鍵とフォールバック鍵が入る() throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = try #require(try CryptoStore.open(files, userID: user)).store
        defer { store.close() }
        let body = try store.registrationRequest()
        #expect(Set(body.keys) == ["device_keys", "one_time_keys", "fallback_keys"])
        guard case .object(let keys) = body["device_keys"] else { Issue.record("device_keys がない"); return }
        #expect(keys["device_id"] == .string(store.deviceID))
        #expect(keys["user_id"] == .string(user))
    }

    @Test func 暗号ストアを壊すとログアウトで消せる() throws {
        let (files, key, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = try #require(try CryptoStore.open(files, userID: user)).store
        try store.destroy(files)
        #expect(try key.load() == nil)
        #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("state.json").path))
    }
}

@MainActor
private final class FakeAPI: E2eeDeviceAPI {
    var registered: [[String: JSONValue]] = []
    var failures: [E2eeAPIError] = []
    var listed: [String] = []
    var deleted: [String] = []

    func registerDevice(_ body: [String: JSONValue]) async throws {
        registered.append(body)
        if !failures.isEmpty { throw failures.removeFirst() }
    }
    func deviceIDs() async throws -> [String] { listed }
    func deleteDevice(_ deviceID: String) async throws { deleted.append(deviceID) }
}

@MainActor
struct DeviceRegistrationTests {
    @Test func 初回は端末名を付けて登録し登録済みにする() async throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let api = FakeAPI()
        let store = try await DeviceRegistration(api: api, files: files, userID: user, displayName: "iPhone（iOS）").ensureRegistered()
        defer { store.close() }
        #expect(store.isRegistered)
        #expect(api.registered.count == 1)
        #expect(api.registered[0]["display_name"] == .string("iPhone（iOS）"))
    }

    @Test func 登録済みなら登録しない() async throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let api = FakeAPI()
        let registration = DeviceRegistration(api: api, files: files, userID: user)
        try await registration.ensureRegistered().close()
        try await registration.ensureRegistered().close()
        #expect(api.registered.count == 1)
    }

    @Test func 登録に失敗したら登録済みにせず同じ鍵で送り直せる() async throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let api = FakeAPI()
        api.failures = [E2eeAPIError(status: 503, code: nil)]
        let registration = DeviceRegistration(api: api, files: files, userID: user)
        await #expect(throws: E2eeAPIError(status: 503, code: nil)) { try await registration.ensureRegistered() }
        try await registration.ensureRegistered().close()
        #expect(api.registered.count == 2)
        #expect(api.registered[0]["device_keys"] == api.registered[1]["device_keys"])
    }

    @Test func 削除済みの端末IDで拒まれたら一覧にないときだけ端末を作り直す() async throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let api = FakeAPI()
        api.failures = [E2eeAPIError(status: 409, code: "E2EE_DEVICE_EXISTS")]
        let store = try await DeviceRegistration(api: api, files: files, userID: user).ensureRegistered()
        defer { store.close() }
        #expect(store.isRegistered)
        #expect(api.registered.count == 2)
        guard case .object(let first) = api.registered[0]["device_keys"], case .object(let second) = api.registered[1]["device_keys"] else {
            Issue.record("device_keys がない"); return
        }
        #expect(first["device_id"] != second["device_id"])
    }

    @Test func 削除済みの拒否でも一覧に今の端末があれば作り直さず失敗にする() async throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let api = FakeAPI()
        api.failures = [E2eeAPIError(status: 409, code: "E2EE_DEVICE_EXISTS")]
        let registration = DeviceRegistration(api: api, files: files, userID: user)
        // 一覧に今の端末がある状態にするため、登録の本文から device_id を読む前に一覧へ入れておく。
        let peek = try #require(try CryptoStore.open(files, userID: user)).store
        api.listed = [peek.deviceID]
        peek.close()
        await #expect(throws: E2eeAPIError.self) { try await registration.ensureRegistered() }
        #expect(api.registered.count == 1)
    }

    @Test func 保存鍵を失ったら前の端末をサーバーから消す() async throws {
        let (files, key, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let api = FakeAPI()
        let registration = DeviceRegistration(api: api, files: files, userID: user)
        let first = try await registration.ensureRegistered()
        let oldID = first.deviceID
        first.close()
        try key.delete()
        let second = try await registration.ensureRegistered()
        defer { second.close() }
        #expect(api.deleted == [oldID])
        #expect(second.deviceID != oldID)
    }

    @Test func 端末名はiPadとそれ以外で分かれる() {
        #expect(DeviceRegistration.defaultDisplayName(idiom: .pad) == "iPad（iOS）")
        #expect(DeviceRegistration.defaultDisplayName(idiom: .phone) == "iPhone（iOS）")
    }
}
