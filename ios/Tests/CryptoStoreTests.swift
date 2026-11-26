import Foundation
import Synchronization
import Testing
@testable import Zoo

final class MemoryKey: StorageKeyBacking, @unchecked Sendable {
    private let lock = NSLock()
    private var value: Data?
    func load() throws -> Data? { lock.withLock { value } }
    func save(_ key: Data) throws { lock.withLock { value = key } }
    func delete() throws { lock.withLock { value = nil } }
}

let user = "11111111-1111-4111-8111-111111111111"

/// 保存だけが失敗する保存鍵の置き場。
struct FailingSaveKey: StorageKeyBacking {
    let base: MemoryKey
    func load() throws -> Data? { try base.load() }
    func save(_ key: Data) throws { throw NSError(domain: NSOSStatusErrorDomain, code: -34018) }
    func delete() throws { try base.delete() }
}

func makeFiles(_ key: MemoryKey = MemoryKey()) -> (CryptoStoreFiles, MemoryKey, URL) {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    return (CryptoStoreFiles(directory: directory, keys: StorageKeyStore(backing: key)), key, directory)
}

struct CryptoStoreTests {
    @Test func 届け先に渡す前の検証メッセージは処理済みの印と同じ状態に保存され古い状態でも読める() throws {
        // 古い `state.json`（項目がない）は、空として読める。
        let old = #"{"version":1,"user_id":"u","device_id":"d","registered":true,"account":"a"}"#
        let decoded = try JSONDecoder().decode(StoreState.self, from: Data(old.utf8))
        #expect(decoded.pending_verifications == [] && decoded.processed_to_device == [])
        // 処理済みの ID と、届け先に渡す前のメッセージは、同じ状態として書いて読める（同じ書き込みで、両方が残るか、どちらも残らない）。
        var state = decoded
        state.processed_to_device = ["row-1"]
        state.pending_verifications = [#"{"type":"m.key.verification.request"}"#]
        let again = try JSONDecoder().decode(StoreState.self, from: JSONEncoder().encode(state))
        #expect(again.processed_to_device == ["row-1"] && again.pending_verifications == state.pending_verifications)
    }

    @Test func 暗号化チャンネルとして見たチャンネルは開き直しても覚えている() throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        var store = try #require(try CryptoStore.open(files, userID: user)).store
        #expect(!store.isRememberedEncrypted("c1"))
        try store.rememberEncryptedChannels(["c1", "c2", "c1"])
        #expect(store.isRememberedEncrypted("c1") && store.isRememberedEncrypted("c2") && !store.isRememberedEncrypted("c3"))
        store.close()
        store = try #require(try CryptoStore.open(files, userID: user)).store
        #expect(store.isRememberedEncrypted("c1") && store.isRememberedEncrypted("c2"))
        store.close()
    }

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
        #expect(second.store.pendingDeviceDeletions.isEmpty)
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
        #expect(second.store.pendingDeviceDeletions == [oldID])
        #expect(second.store.deviceID != oldID)
        second.store.close()
    }

    @Test func 作り直しの途中で失敗しても前の端末のIDを失わない() throws {
        let key = MemoryKey()
        let (files, _, directory) = makeFiles(key)
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = try #require(try CryptoStore.open(files, userID: user)).store
        let oldID = first.deviceID
        first.close()
        try key.delete()
        // 新しい保存鍵を Keychain に書けずに失敗する（前の `state.json` はそのまま残る）。
        let failing = CryptoStoreFiles(directory: files.directory, keys: StorageKeyStore(backing: FailingSaveKey(base: key)))
        #expect(throws: (any Error).self) { try CryptoStore.open(failing, userID: user) }
        // 開き直すと、前の端末の ID を取り出せる。作り直した端末の状態にも残り、消せたときだけ記録から外す。
        let second = try #require(try CryptoStore.open(files, userID: user)).store
        #expect(second.pendingDeviceDeletions == [oldID])
        second.close()
        let third = try #require(try CryptoStore.open(files, userID: user)).store
        #expect(third.pendingDeviceDeletions == [oldID])
        try third.clearPendingDeletion(oldID)
        #expect(third.pendingDeviceDeletions.isEmpty)
        third.close()
    }

    @Test func 復旧中に状態ファイルの保存が失敗しても開き直すと前の端末のIDを取り出せる() throws {
        let key = MemoryKey()
        let (files, _, directory) = makeFiles(key)
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = try #require(try CryptoStore.open(files, userID: user)).store
        let oldID = first.deviceID
        first.close()
        // 保存鍵を失った状態で、復旧の最後の `state.json` の保存だけを失敗させる（一時ファイルの場所をディレクトリーにする）。
        try key.delete()
        let blocker = directory.appendingPathComponent("state.json.tmp", isDirectory: true)
        try FileManager.default.createDirectory(at: blocker, withIntermediateDirectories: true)
        #expect(throws: (any Error).self) { try CryptoStore.open(files, userID: user) }
        // 前の `state.json` は、そのまま残っている。
        let url = directory.appendingPathComponent("state.json")
        let stored = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
        #expect(stored["device_id"] as? String == oldID)
        // 保存できるようになってから開き直すと、前の端末の ID を取り出せる。
        try FileManager.default.removeItem(at: blocker)
        let second = try #require(try CryptoStore.open(files, userID: user)).store
        #expect(second.pendingDeviceDeletions == [oldID])
        #expect(second.deviceID != oldID)
        second.close()
    }

    @Test func 復旧中にrenameが失敗しても開き直すと前の端末のIDを取り出せる() throws {
        let key = MemoryKey()
        let (files, _, directory) = makeFiles(key)
        let url = directory.appendingPathComponent("state.json")
        defer {
            try? FileManager.default.setAttributes([.immutable: false], ofItemAtPath: url.path)
            try? FileManager.default.removeItem(at: directory)
        }
        let first = try #require(try CryptoStore.open(files, userID: user)).store
        let oldID = first.deviceID
        first.close()
        try key.delete()
        // 既存の `state.json` を変更できない（不変）ようにして、一時ファイルは書けるが、`rename` だけが失敗する状態にする。
        try FileManager.default.setAttributes([.immutable: true], ofItemAtPath: url.path)
        do {
            _ = try CryptoStore.open(files, userID: user)
            Issue.record("rename の失敗で投げるはず")
        } catch {
            // `rename(2)` の失敗（`POSIXError`）であって、一時ファイルの書き込みなど、ほかの失敗ではない。
            #expect(error is POSIXError)
        }
        #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("state.json.tmp").path))
        // 前の `state.json` は、そのまま残っている。
        let stored = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
        #expect(stored["device_id"] as? String == oldID)
        // 置き換えられるようになってから開き直すと、前の端末の ID を取り出せる。
        try FileManager.default.setAttributes([.immutable: false], ofItemAtPath: url.path)
        let second = try #require(try CryptoStore.open(files, userID: user)).store
        #expect(second.pendingDeviceDeletions == [oldID])
        second.close()
    }

    @Test func Olmセッションの復元に失敗しても作り直さず投げる() throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = try #require(try CryptoStore.open(files, userID: user)).store
        let deviceID = first.deviceID
        first.close()
        // 保存鍵は合っているが、Olm セッションの pickle だけが壊れている状態にする。
        let url = directory.appendingPathComponent("state.json")
        var object = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
        object["olm_sessions"] = [["their_identity_key": "x", "session_id": "y", "pickle": "壊れている"]]
        try JSONSerialization.data(withJSONObject: object).write(to: url)
        #expect(throws: (any Error).self) { try CryptoStore.open(files, userID: user) }
        // 保存鍵も状態も消さず、前の端末の ID も記録しない（受信用セッションを失わない）。
        #expect(!CryptoStore.isReserved(directory))
        let stored = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
        #expect(stored["device_id"] as? String == deviceID)
        #expect((stored["pending_deletions"] as? [String] ?? []).isEmpty)
    }

    @Test func 初期保存に失敗した直後に別のopenが予約を取っても外さない() throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        // 初期保存を失敗させる：一時ファイルの場所をディレクトリーにしておく。
        let blocker = directory.appendingPathComponent("state.json.tmp", isDirectory: true)
        try FileManager.default.createDirectory(at: blocker, withIntermediateDirectories: true)
        let other = Mutex<CryptoStore?>(nil)
        // 作りかけのインスタンスの `deinit` が予約を外したあと、`open` の後始末の前に、別の `open` が予約を取る。
        let path = directory.standardizedFileURL.path
        CryptoStore.openFailureHooks.withLock {
            $0[path] = {
                try? FileManager.default.removeItem(at: blocker)
                other.withLock { $0 = try? CryptoStore.open(files, userID: user)?.store }
            }
        }
        defer { CryptoStore.openFailureHooks.withLock { $0[path] = nil } }
        // 最初の失敗は、初期保存によるもの（予約の競合ではない）。
        do {
            _ = try CryptoStore.open(files, userID: user)
            Issue.record("初期保存の失敗で投げるはず")
        } catch {
            #expect(!(error is CryptoStoreBusyError))
        }
        let second = try #require(other.withLock { $0 })
        // 後始末は、別の持ち主の予約を外さない：まだ予約されていて、同じストアを重ねて開けない。
        #expect(CryptoStore.isReserved(directory))
        #expect(throws: CryptoStoreBusyError.self) { try CryptoStore.open(files, userID: user) }
        second.close()
        #expect(!CryptoStore.isReserved(directory))
    }

    @Test func 置き換えに失敗しても元のファイルを動かさず一時ファイルを残さない() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        // `state.json` が、中身のあるディレクトリーになっている（ファイルでは置き換えられない）状態にして、置き換えを失敗させる。
        let state = directory.appendingPathComponent("state.json", isDirectory: true)
        try FileManager.default.createDirectory(at: state, withIntermediateDirectories: true)
        let original = state.appendingPathComponent("前の記録")
        try Data("前の端末の記録".utf8).write(to: original)
        #expect(throws: (any Error).self) { try CryptoStore.writeAtomically(Data("新しい".utf8), to: state) }
        // 元のものは、その場所にそのまま残る。一時ファイルも残らない。
        #expect(try String(contentsOf: original, encoding: .utf8) == "前の端末の記録")
        #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("state.json.tmp").path))
    }

    @Test func 状態ファイルの書き込みは成功すれば置き換わり一時ファイルを残さない() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let state = directory.appendingPathComponent("state.json")
        try CryptoStore.writeAtomically(Data("1".utf8), to: state)
        try CryptoStore.writeAtomically(Data("2".utf8), to: state)
        #expect(try String(contentsOf: state, encoding: .utf8) == "2")
        #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("state.json.tmp").path))
    }

    @Test func 予約は持ち主の印が合うときだけ外れる() throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = try #require(try CryptoStore.open(files, userID: user)).store
        // 別の持ち主の解除は、予約を外さない（作りかけの後始末が、あとから開いた別のインスタンスの予約を外さない）。
        CryptoStore.releaseReservation(directory, token: UUID())
        #expect(CryptoStore.isReserved(directory))
        #expect(throws: CryptoStoreBusyError.self) { try CryptoStore.open(files, userID: user) }
        store.close()
        #expect(!CryptoStore.isReserved(directory))
    }

    @Test func 閉じた暗号ストアは使えず消せない() throws {
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = try #require(try CryptoStore.open(files, userID: user)).store
        store.close()
        #expect(throws: E2eeStoreError.closed) { try store.markRegistered() }
        #expect(throws: E2eeStoreError.closed) { try store.acknowledged(["1"]) }
        // 閉じたインスタンスは、開き直した暗号ストアを消せない。
        let reopened = try #require(try CryptoStore.open(files, userID: user)).store
        #expect(throws: E2eeStoreError.closed) { try store.destroy(files) }
        #expect(FileManager.default.fileExists(atPath: directory.appendingPathComponent("state.json").path))
        try reopened.destroy(files)
        // 消し終えたあとは、新しく開ける。
        try #require(try CryptoStore.open(files, userID: user)).store.close()
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
