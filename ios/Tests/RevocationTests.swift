import Foundation
import Testing
import ZooCrypto
@testable import Zoo

private func text(_ value: JSONValue) -> String { String(decoding: try! JSONEncoder().encode(value), as: UTF8.self) }
private func parse(_ json: String) -> [String: JSONValue] {
    guard case .object(let value) = try! JSONDecoder().decode(JSONValue.self, from: Data(json.utf8)) else { fatalError() }
    return value
}

/// 失効の API を模した、状態を持つサーバー（初回設定のあとの状態から始める）。
@MainActor
private final class RevocationServer: E2eeRevocationAPI {
    var storage: [String: JSONValue]
    var keys: [String: JSONValue]
    var signatures: [JSONValue]
    var versions: [JSONValue]
    var deleted: [String] = []
    var requests: [String] = []
    var reauthSeen: [String?] = []
    var requireReauth = true
    /// 次の 1 回だけ、このパスの要求を通信エラーにする（途中で止めて再開するテスト）。
    var failOnce: String?
    var loseStorageResponse = false
    var backupPages: [String: [JSONValue]] = [:]
    var backupTrace: [String] = []
    var uploadedRows: [JSONValue] = []
    var uploadFailure: Int?

    init(from server: FakeSetupServer, userID: String) async throws {
        storage = parse(try await server.secretStorage())
        keys = parse(try await server.queryOwnKeys(userID: userID))
        signatures = keys["signatures"]?.array ?? []
        versions = parse(try await server.backupVersions())["versions"]?.array ?? []
    }

    func call(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?, reauth: String?,
              headers: [String: String]) async throws -> JSONValue {
        requests.append("\(method) \(path)")
        let guardedWrite = method != "GET" && method != "DELETE" && path != "api/e2ee/keys/query"
        if guardedWrite, requireReauth, reauth == nil, path != "api/e2ee/key_backup/keys" { throw E2eeAPIError(status: 403, code: "E2EE_REAUTH_REQUIRED") }
        reauthSeen.append(reauth)
        if failOnce == path { failOnce = nil; throw E2eeAPIError(status: 500, code: nil) }
        switch (method, path) {
        case ("POST", "api/e2ee/secret_storage/keys"):
            // 全範囲：サーバーの今の既定の鍵と版が前提のときだけ、新しい SSSS 鍵に作り直す（鍵スロットも置き換える）。
            guard body?["expected_default_key_id"]?.string == storage["default_key_id"]?.string,
                  body?["expected_secrets_revision"]?.uint == storage["secrets_revision"]?.uint else {
                throw E2eeAPIError(status: 409, code: "E2EE_SSSS_CONFLICT")
            }
            let keyID = body?["key_id"] ?? .null
            var secrets = body?["secrets"]?.object ?? [:]
            for (name, secret) in secrets {
                guard case .object(var value) = secret else { continue }
                value["key_id"] = keyID
                secrets[name] = .object(value)
            }
            let revision = (storage["secrets_revision"]?.uint ?? 0) + 1
            storage = ["default_key_id": keyID, "keys": .object([keyID.string ?? "": body?["key"] ?? .null]),
                       "secrets": .object(secrets), "secrets_revision": .number(Double(revision)),
                       "slots": body?["slots"] ?? .array([])]
            if loseStorageResponse { loseStorageResponse = false; throw URLError(.networkConnectionLost) }
            return .object(["secrets_revision": .number(Double(revision))])
        case ("GET", "api/e2ee/secret_storage"): return .object(storage)
        case ("POST", "api/e2ee/secret_storage/secrets"):
            guard body?["expected_secrets_revision"]?.uint == storage["secrets_revision"]?.uint else {
                throw E2eeAPIError(status: 409, code: "E2EE_SSSS_CONFLICT")
            }
            let revision = (storage["secrets_revision"]?.uint ?? 0) + 1
            var secrets = storage["secrets"]?.object ?? [:]
            for (name, secret) in body?["secrets"]?.object ?? [:] {
                guard case .object(var value) = secret else { continue }
                value["key_id"] = storage["default_key_id"]
                secrets[name] = .object(value)
            }
            storage["secrets"] = .object(secrets)
            storage["secrets_revision"] = .number(Double(revision))
            if loseStorageResponse { loseStorageResponse = false; throw URLError(.networkConnectionLost) }
            return .object(["secrets_revision": .number(Double(revision))])
        case ("POST", "api/e2ee/cross_signing/keys"):
            for (field, target) in [("master_key", "master_keys"), ("self_signing_key", "self_signing_keys"), ("user_signing_key", "user_signing_keys")] {
                if let key = body?[field] { keys[target] = .object([storage["user_id"]?.string ?? userOf(keys): key]) }
            }
            return .null
        case ("POST", "api/e2ee/cross_signing/signatures"):
            let owner = userOf(keys)
            for case .object(let upload) in body?["signatures"]?.array ?? [] {
                signatures.append(.object(["signer_user_id": .string(owner), "signer_key_id": upload["signer_key_id"]!,
                                           "target_user_id": upload["target_user_id"]!, "target_key_id": upload["target_key_id"]!,
                                           "signature": upload["signature"]!]))
            }
            return .null
        case ("POST", "api/e2ee/keys/query"):
            var response = keys
            response["signatures"] = .array(signatures)
            return .object(response)
        case ("GET", "api/e2ee/key_backup/versions"): return .object(["versions": .array(versions)])
        case ("POST", "api/e2ee/key_backup/versions"):
            let number = String(versions.count + 1)
            versions.append(.object(["version": .string(number), "algorithm": body?["algorithm"] ?? .null,
                                     "auth_data": body?["auth_data"] ?? .null, "deleted": .bool(false)]))
            return .object(["version": .string(number)])
        case ("GET", "api/e2ee/key_backup/versions/current"):
            guard let current = versions.last(where: { $0.object?["deleted"] != .bool(true) }) else { throw E2eeAPIError(status: 404, code: nil) }
            return current
        case ("GET", "api/e2ee/tofu_pins"): return .object(["pins": .array([]), "next": .null])
        case ("GET", "api/e2ee/key_backup/keys"):
            let cursor = query["after"] ?? "first"
            backupTrace.append("get:\(cursor)")
            let rows = backupPages[cursor] ?? []
            return .object(["sessions": .array(rows), "next": cursor == "first" && backupPages["second"] != nil ? .string("second") : .null])
        case ("POST", "api/e2ee/key_backup/keys"):
            backupTrace.append("post")
            if let uploadFailure { throw E2eeAPIError(status: uploadFailure, code: nil) }
            uploadedRows += body?["sessions"]?.array ?? []
            return .null
        default:
            if method == "DELETE", path.hasPrefix("api/e2ee/key_backup/versions/") {
                let version = String(path.dropFirst("api/e2ee/key_backup/versions/".count))
                backupTrace.append("delete:\(version)")
                deleted.append(version)
                versions = versions.map { value in
                    guard case .object(var object) = value, object["version"]?.string == version else { return value }
                    object["deleted"] = .bool(true)
                    return .object(object)
                }
                return .null
            }
            if method == "GET", path.hasPrefix("api/e2ee/key_backup/versions/") {
                let version = String(path.dropFirst("api/e2ee/key_backup/versions/".count))
                return versions.first { $0.object?["version"]?.string == version } ?? .null
            }
            throw E2eeAPIError(status: 404, code: nil)
        }
    }

    private func userOf(_ keys: [String: JSONValue]) -> String { keys["master_keys"]?.object?.keys.first ?? "" }
}

@MainActor
struct RevocationTests {
    /// 初回設定を済ませた端末と、その状態から始まる模擬サーバーと、開けた SSSS。
    private func prepared() async throws -> (Device, RevocationServer, UnlockedSecretStorage, String) {
        let fake = FakeSetupServer(userID: user)
        let device = try Device(fake)
        let start = try device.setup.begin()
        try await device.setup.completeSetup(start.session, reauth: "t")
        let server = try await RevocationServer(from: fake, userID: user)
        let storage = try #require(try parseSecretStorage(userId: user, response: try await fake.secretStorage()))
        let unlocked = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: start.recoveryKey))
        return (device, server, unlocked, start.recoveryKey)
    }

    @Test("失効の保存失敗後は再読込した記録から再開する", arguments: [false, true])
    func resumesDurableRevocationAfterSaveFailure(afterRename: Bool) async throws {
        let (device, server, unlocked, recoveryKey) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await revocation.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        let path = device.files.directory.standardizedFileURL.path
        CryptoStore.writeFailureHooks.withLock { hooks in hooks[path] = { url, renamed in
            if url.lastPathComponent == CryptoStore.stateFile, renamed == afterRename { throw CocoaError(.fileWriteUnknown) }
        } }
        defer { CryptoStore.writeFailureHooks.withLock { $0.removeValue(forKey: path) } }
        await #expect(throws: (any Error).self) { try await revocation.run(reauth: "token") }
        #expect(!device.store.isUsable)
        CryptoStore.writeFailureHooks.withLock { $0.removeValue(forKey: path) }
        try device.restart()
        let requestsBeforeRetry = server.requests.count
        await #expect(throws: RevocationError.none) { try await revocation.run(reauth: "token") }
        #expect(server.requests.count == requestsBeforeRetry)
        let current = try #require(try parseSecretStorage(userId: user, response: text(.object(server.storage))))
        _ = try revocation.resume(unlocked: current.unlockWithRecoveryKey(recoveryKey: RecoveryKey.parse(input: recoveryKey)))
        let finished = try await revocation.run(reauth: "token")
        #expect(finished.stage == "verified")
        #expect(try revocation.savedSummary() == nil)
    }

    @Test("書込みAPIのtxid付き応答からdataを取り出す")
    func unwrapsMutationEnvelope() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in (200, [:], Data(#"{"data":{"version":"2"},"txid":null}"#.utf8)) }
        let result = try await harness.api.call("api/e2ee/key_backup/versions", method: "POST", query: [:],
            body: [:], reauth: nil, headers: [:])
        #expect(result.object?["version"]?.string == "2")
    }

    @Test("失効適用中の保存失敗後は同じストアを再利用しない", arguments: [false, true])
    func applyRevocationSaveFailure(afterRename: Bool) async throws {
        let (device, server, unlocked, _) = try await prepared()
        defer { device.cleanUp() }
        let controller = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await controller.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        let revocation = try device.store.resumeRevocation(unlocked: unlocked)
        let path = device.files.directory.standardizedFileURL.path
        CryptoStore.writeFailureHooks.withLock { hooks in hooks[path] = { _, renamed in
            if renamed == afterRename { throw CocoaError(.fileWriteUnknown) }
        } }
        defer { CryptoStore.writeFailureHooks.withLock { $0.removeValue(forKey: path) } }
        #expect(throws: (any Error).self) { try device.store.applyRevocation(revocation, secretsRevision: 2) }
        #expect(!device.store.isUsable)
        #expect(throws: E2eeStoreError.closed) { try device.store.applyRevocation(revocation, secretsRevision: 2) }
        CryptoStore.writeFailureHooks.withLock { $0.removeValue(forKey: path) }
        try device.restart()
        let resumed = try device.store.resumeRevocation(unlocked: unlocked)
        try device.store.applyRevocation(resumed, secretsRevision: 2)
        #expect(device.store.isUsable && device.store.secretsRevision == 2)
    }

    @Test func 失効の信頼適用が保存前に失敗してもストアを再利用しない() async throws {
        let (device, server, unlocked, _) = try await prepared()
        defer { device.cleanUp() }
        let controller = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await controller.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        let revocation = try device.store.resumeRevocation(unlocked: unlocked)
        let (files, _, directory) = makeFiles()
        defer { try? FileManager.default.removeItem(at: directory) }
        let otherUser = UUID().uuidString.lowercased()
        let other = try #require(try CryptoStore.open(files, userID: otherUser)).store
        defer { other.close() }
        let url = directory.appendingPathComponent(CryptoStore.stateFile)
        let saved = try Data(contentsOf: url)
        #expect(throws: (any Error).self) { try other.applyRevocation(revocation, secretsRevision: 2) }
        #expect(!other.isUsable)
        #expect(try Data(contentsOf: url) == saved)
        #expect(throws: E2eeStoreError.closed) { try other.applyRevocation(revocation, secretsRevision: 2) }
        other.close()
        let reopened = try #require(try CryptoStore.open(files, userID: otherUser)).store
        defer { reopened.close() }
        #expect(reopened.isUsable && reopened.secretsRevision == nil)
    }

    @Test func 端末だけの失効は鍵を作り直して保存し記録を消す() async throws {
        let (device, server, unlocked, firstRecoveryKey) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        let oldMaster = server.keys["master_keys"]
        let started = try await revocation.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        #expect(started.stage == "prepared")
        // 始めた時点で、進み具合が暗号ストアにある（アプリを閉じても続けられる）。
        #expect(try revocation.savedSummary()?.revocation_id == started.revocation_id)
        // 終わっていない失効があるうちは、新しい失効を始めない。
        await #expect(throws: RevocationError.pending) {
            _ = try await revocation.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        }

        // 再認証なしでは、保存の最初の要求が 403 になる（何も保存されない）。
        await #expect(throws: E2eeSetupError.reauthRequired) { try await revocation.run(reauth: nil) }
        #expect(try revocation.savedSummary()?.stage == "prepared")

        let finished = try await revocation.run(reauth: "token")
        #expect(finished.stage == "verified")
        #expect(try revocation.savedSummary() == nil)
        // 端末だけの失効は、マスター鍵を残し、SSSS の秘密を置き換え（版が進む）、新しい版を作って古い版を消す。
        #expect(server.keys["master_keys"] == oldMaster)
        #expect(server.storage["secrets_revision"]?.uint == 2)
        #expect(server.versions.count == 2)
        #expect(server.deleted == ["1"])
        #expect(server.requests.contains("POST api/e2ee/cross_signing/signatures"))
        // 暗号ストアの自分の鍵は、保存した新しい鍵にそろい、新しい SSSS の版を覚えている。
        #expect(device.store.secretsRevision == 2)
    }

    @Test func 端末だけの失効は保存応答を失っても再起動後に同じ書込みから再開する() async throws {
        let (device, server, unlocked, recoveryKey) = try await prepared()
        defer { device.cleanUp() }
        let controller = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await controller.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        server.loseStorageResponse = true
        await #expect(throws: URLError.self) { try await controller.run(reauth: "token") }
        #expect(server.storage["secrets_revision"]?.uint == 2)
        try device.restart()
        let current = try #require(try parseSecretStorage(userId: user, response: text(.object(server.storage))))
        let retry = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try retry.resume(unlocked: current.unlockWithRecoveryKey(recoveryKey: RecoveryKey.parse(input: recoveryKey)))
        #expect(try await retry.run(reauth: "token").stage == "verified")
        #expect(server.storage["secrets_revision"]?.uint == 2)
    }

    @Test("旧バックアップはページの送信成功後に次ページを読み最後に削除する", arguments: [404, 500])
    func migratesBackupPageByPage(failedStatus: Int) async throws {
        let (device, server, unlocked, _) = try await prepared()
        defer { device.cleanUp() }
        let capture = CapturingRequests(user: user, store: { device.store }, setupServer: device.server)
        try await MessageEncryption(store: { device.store }, api: capture).send(
            channel: UUID().uuidString.lowercased(), id: UUID().uuidString.lowercased(), parent: nil, body: "移行する本文")
        _ = try await device.setup.uploadKeyBackup()
        try await MessageEncryption(store: { device.store }, api: capture).send(
            channel: UUID().uuidString.lowercased(), id: UUID().uuidString.lowercased(), parent: nil, body: "別セッションの本文")
        _ = try await device.setup.uploadKeyBackup()
        let rows = Array(device.server.backupRows.values)
        try #require(rows.count == 2)
        let sessionIDs = Set(rows.compactMap { $0.object?["session_id"]?.string })
        try #require(sessionIDs.count == 2)
        server.backupPages = ["first": [rows[0]], "second": [rows[1]]]
        let controller = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await controller.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        server.uploadFailure = failedStatus
        await #expect(throws: E2eeAPIError.self) { try await controller.run(reauth: "token") }
        #expect(server.backupTrace == ["get:first", "post"])
        #expect(server.deleted.isEmpty && server.uploadedRows.isEmpty)
        server.uploadFailure = nil
        server.backupTrace = []
        #expect(try await controller.run(reauth: "token").stage == "verified")
        #expect(server.backupTrace == ["get:first", "post", "get:second", "post", "delete:1"])
        #expect(server.uploadedRows.count == 2 && server.deleted == ["1"])
        #expect(Set(server.uploadedRows.compactMap { $0.object?["session_id"]?.string }) == sessionIDs)
    }

    @Test func 全範囲の失効は新しいSSSS鍵とマスター鍵を作り直し古いリカバリーキーを使えなくする() async throws {
        let (device, server, unlocked, firstRecoveryKey) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        let oldKeyID = server.storage["default_key_id"]?.string
        let oldMaster = server.keys["master_keys"]
        _ = try await revocation.begin(scope: .full, deletedDevices: [], unlocked: unlocked)
        // 鍵スロットがないと、保存の要求を作れない。
        await #expect(throws: (any Error).self) { try await revocation.run(reauth: "token") }
        let recoveryKey = try revocation.newRecoveryKey()

        let finished = try await revocation.run(reauth: "token")
        #expect(finished.stage == "verified")
        #expect(try revocation.savedSummary() == nil)
        // 新しい SSSS 鍵（版が進む）、新しいマスター鍵、新しい版（古い版は消える）。
        #expect(server.storage["default_key_id"]?.string != oldKeyID)
        #expect(server.storage["secrets_revision"]?.uint == 2)
        #expect(server.keys["master_keys"] != oldMaster)
        #expect(server.storage["slots"]?.array?.count == 1)
        #expect(server.versions.count == 2 && server.deleted == ["1"])
        #expect(device.store.secretsRevision == 2)
        // 新しいリカバリーキーだけが、新しい SSSS を開ける。
        let storage = try #require(try parseSecretStorage(userId: user, response: text(.object(server.storage))))
        _ = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: recoveryKey))
        let old = try RecoveryKey.parse(input: firstRecoveryKey)
        #expect(throws: (any Error).self) { _ = try storage.unlockWithRecoveryKey(recoveryKey: old) }
    }

    @Test func 端末を作り直すと途中の失効の記録を捨て破棄したことを再起動後も覚えて確認すると消える() async throws {
        let (device, server, unlocked, _) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await revocation.begin(scope: .deviceOnly, deletedDevices: [], unlocked: unlocked)
        #expect(try revocation.savedSummary() != nil && !device.store.revocationDropped)

        // 端末が別の端末から削除されて作り直された。古い端末アカウントで封をした失効の記録は開けないので、捨てる。
        try device.store.replaceDevice()
        #expect(try revocation.savedSummary() == nil)
        #expect(device.store.revocationDropped)
        // アプリを開き直しても、破棄したことを覚えている。
        try device.restart()
        #expect(device.store.revocationDropped)
        // 確認すると消え、開き直しても出ない。
        try device.store.acknowledgeRevocationDropped()
        #expect(!device.store.revocationDropped)
        try device.restart()
        #expect(!device.store.revocationDropped)
        // 失効の記録がなかった端末は、作り直しても、破棄したとは言わない。
        try device.store.replaceDevice()
        #expect(!device.store.revocationDropped)
    }

    @Test func 全範囲をSSSSの保存の前に止めたら最初からやり直す() async throws {
        let (device, server, unlocked, _) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await revocation.begin(scope: .full, deletedDevices: [], unlocked: unlocked)
        // アプリを閉じて開き直した（メモリーの秘密は消え、進み具合だけが暗号ストアにある）。
        try device.restart()
        let reopened = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        #expect(try reopened.savedSummary()?.stage == "prepared")
        #expect {
            _ = try reopened.resume(unlocked: unlocked)
        } throws: { error in
            if case ZooCryptoError.RevocationRestart = error { return true }
            return false
        }
        // 捨てれば、新しい失効を始められる。
        try reopened.abandon()
        #expect(try reopened.savedSummary() == nil)
    }

    @Test func 全範囲をクロス署名鍵の保存で止めても新しいSSSSを開け直して続きから終える() async throws {
        let (device, server, unlocked, _) = try await prepared()
        defer { device.cleanUp() }
        let revocation = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        _ = try await revocation.begin(scope: .full, deletedDevices: [], unlocked: unlocked)
        let recoveryKey = try revocation.newRecoveryKey()
        server.failOnce = "api/e2ee/cross_signing/keys"
        await #expect(throws: (any Error).self) { try await revocation.run(reauth: "token") }
        // SSSS は新しい鍵で保存済みで、進み具合は暗号ストアにある。
        #expect(try revocation.savedSummary()?.stage == "secret_storage_saved")

        // アプリを開き直して、新しいリカバリーキーで SSSS を開け直し、続きから終える。
        try device.restart()
        let reopened = E2eeRevocation(store: { device.store }, api: server, setup: device.setup, userID: user)
        let storage = try #require(try parseSecretStorage(userId: user, response: text(.object(server.storage))))
        let again = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: recoveryKey))
        let resumed = try reopened.resume(unlocked: again)
        #expect(resumed.reached("secret_storage_saved") && !resumed.reached("cross_signing_saved"))
        let finished = try await reopened.run(reauth: "token")
        #expect(finished.stage == "verified")
        #expect(try reopened.savedSummary() == nil)
        #expect(server.versions.count == 2)
    }
}

@MainActor
struct ReverifyTests {
    private let other = "22222222-2222-4222-8222-222222222222"

    @Test func 相手のマスター鍵を受け入れるとピンと署名を作り保存できたら登録待ちを外す() async throws {
        let fake = FakeSetupServer(userID: user)
        let device = try Device(fake)
        defer { device.cleanUp() }
        // 解錠していない端末は、受け入れられない。
        let theirs = try CrossSigningKeys.generate(userId: other)
        let master = theirs.master().publicKeyJson()
        #expect(throws: E2eeStoreError.sendNeedsUnlock) { _ = try device.store.acceptMasterKey(userID: other, masterKey: master) }

        let start = try device.setup.begin()
        try await device.setup.completeSetup(start.session, reauth: "t")
        let accepted = try device.store.acceptMasterKey(userID: other, masterKey: master)
        #expect(accepted.signature.targetUserId == other)
        #expect(accepted.pin.contains(other))
        try device.store.confirmAcceptedMasterKey(userID: other, masterKey: master)
        // 記録のある相手は、失効で署名し直す候補になる（自分は含まない）。
        #expect(device.store.recordedUsers() == [other])
    }
}

@MainActor
struct KeySlotsTests {
    /// 鍵スロットの API を模した、状態を持つサーバー。
    private final class Server: E2eeRevocationAPI {
        var storage: JSONValue
        var reauthSeen: [String?] = []
        var failReauthOnce = false
        init(storage: JSONValue) { self.storage = storage }

        func call(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?, reauth: String?,
                  headers: [String: String]) async throws -> JSONValue {
            var object = storage.object ?? [:]
            var slots = object["slots"]?.array ?? []
            switch (method, path) {
            case ("GET", "api/e2ee/secret_storage"): return storage
            case ("POST", "api/e2ee/secret_storage/slots"):
                reauthSeen.append(reauth)
                if failReauthOnce { failReauthOnce = false; throw E2eeAPIError(status: 403, code: "E2EE_REAUTH_REQUIRED") }
                var slot = body ?? [:]
                slot["created_at"] = .string("2026-11-26T00:00:00Z")
                // 本番と同じく、同じ鍵のリカバリースロットを置き換える。
                if slot["kind"]?.string == "recovery" {
                    slots.removeAll { $0.object?["kind"]?.string == "recovery" && $0.object?["key_id"] == slot["key_id"] }
                }
                slots.append(.object(slot))
            case ("DELETE", _) where path.hasPrefix("api/e2ee/secret_storage/slots/"):
                reauthSeen.append(reauth)
                let id = String(path.dropFirst("api/e2ee/secret_storage/slots/".count))
                slots.removeAll { $0.object?["id"]?.string == id }
            default: throw E2eeAPIError(status: 404, code: nil)
            }
            object["slots"] = .array(slots)
            storage = .object(object)
            return .null
        }
    }

    private func prepared() async throws -> (Server, UnlockedSecretStorage, String, E2eeKeySlots) {
        let fake = FakeSetupServer(userID: user)
        let device = try Device(fake)
        let start = try device.setup.begin()
        try await device.setup.completeSetup(start.session, reauth: "t")
        let server = Server(storage: try JSONDecoder().decode(JSONValue.self, from: Data(try await fake.secretStorage().utf8)))
        let parsed = try #require(try parseSecretStorage(userId: user, response: try await fake.secretStorage()))
        let unlocked = try parsed.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: start.recoveryKey))
        device.cleanUp()
        return (server, unlocked, start.recoveryKey, E2eeKeySlots(api: server, userID: user))
    }

    @Test func リカバリーキーを作り直すと新しいキーだけが解錠でき古いキーは使えなくなる() async throws {
        let (server, unlocked, firstKey, slots) = try await prepared()
        try await slots.load()
        #expect(slots.slots.count == 1 && slots.slots[0].kind == "recovery")

        let prepared = try slots.prepareRecovery(unlocked: unlocked)
        #expect(server.reauthSeen.isEmpty)
        let before = try #require(try parseSecretStorage(userId: user, response: String(decoding: try JSONEncoder().encode(server.storage), as: UTF8.self)))
        _ = try before.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: firstKey))
        try await slots.saveRecovery(prepared, reauth: { "token" })
        try await slots.load()
        let newKey = prepared.display
        // 置き換えは再認証つきの POST だけで行い、リカバリースロットは 1 つのまま。
        #expect(server.reauthSeen == ["token"])
        #expect(slots.slots.count == 1 && slots.slots[0].kind == "recovery")
        let storage = try #require(try parseSecretStorage(userId: user, response: String(decoding: try JSONEncoder().encode(server.storage), as: UTF8.self)))
        _ = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: newKey))
        #expect(throws: (any Error).self) { _ = try storage.unlockWithRecoveryKey(recoveryKey: try RecoveryKey.parse(input: firstKey)) }
        // 最後の 1 つは消せない。
        await #expect(throws: KeySlotError.lastSlot) { try await slots.remove(slots.slots[0], reauth: { "token3" }) }
    }

    @Test func 再認証が足りなければ取り直してもう一度だけ行う() async throws {
        let (server, unlocked, _, slots) = try await prepared()
        try await slots.load()
        server.failReauthOnce = true
        var tokens = 0
        let prepared = try slots.prepareRecovery(unlocked: unlocked)
        try await slots.saveRecovery(prepared, reauth: { tokens += 1; return "token\(tokens)" })
        try await slots.load()
        // 追加の再認証が足りなければ、取り直して 1 回だけ再試行する。
        #expect(server.reauthSeen == ["token1", "token2"] && slots.slots.count == 1)
    }
}
