import Foundation
import Observation
import ZooCrypto

private func sasJSONText(_ value: JSONValue) -> String { String(decoding: (try? JSONEncoder().encode(value)) ?? Data(), as: UTF8.self) }

/// 検証で使う API（docs/e2ee.md の「絵文字 SAS 検証」）。
@MainActor
protocol E2eeVerificationAPI {
    /// 相手の `keys/query`（1 人）。端末鍵、クロス署名鍵、署名、ピンをまとめた JSON。
    func queryKeys(userID: String) async throws -> String
    /// 複数のユーザーの `keys/query`（`user_ids` に、判定するユーザーを入れた JSON）。
    func queryKeys(userIDs: [String]) async throws -> String
    /// TOFU のピンを登録する。別の端末のピンと食い違えば（409 `E2EE_PIN_CONFLICT`）`false`。
    func registerPin(_ pin: String) async throws -> Bool
    /// `keys/claim`。応答の `one_time_keys`。
    func claim(_ oneTimeKeys: [String: JSONValue]) async throws -> [String: JSONValue]
    /// `to_device`。配れなかった端末の数（応答の `undelivered`）。
    func sendToDevice(deviceID: String, messages: [String: JSONValue]) async throws -> Int
    /// 受け入れたマスター鍵のピン（`replace: true`）と署名を、直前の再認証つきで保存する。
    func saveAccepted(pin: String, signature: SignatureUpload, reauth: String) async throws
}

extension APIClient: E2eeVerificationAPI {
    func queryKeys(userID: String) async throws -> String {
        let data = try await e2ee("api/e2ee/keys/query", method: "POST", body: ["user_ids": .array([.string(userID)])])
        guard case .object(var response) = try JSONDecoder().decode(JSONValue.self, from: data) else { throw ZooError.invalidResponse }
        if case .object(let envelope)? = response["data"] { response = envelope }
        response["user_ids"] = .array([.string(userID)])
        return String(decoding: try JSONEncoder().encode(JSONValue.object(response)), as: UTF8.self)
    }

    func registerPin(_ pin: String) async throws -> Bool {
        let value = try JSONDecoder().decode(JSONValue.self, from: Data(pin.utf8))
        do {
            _ = try await e2ee("api/e2ee/tofu_pins", method: "POST", body: ["pins": .array([value]), "replace": .bool(false)])
            return true
        } catch let failure as E2eeAPIError where failure.code == "E2EE_PIN_CONFLICT" { return false }
    }

    func claim(_ oneTimeKeys: [String: JSONValue]) async throws -> [String: JSONValue] {
        let data = try await e2ee("api/e2ee/keys/claim", method: "POST", body: ["one_time_keys": .object(oneTimeKeys)])
        guard case .object(var response) = try JSONDecoder().decode(JSONValue.self, from: data) else { throw ZooError.invalidResponse }
        if case .object(let envelope)? = response["data"] { response = envelope }
        return response["one_time_keys"]?.object ?? [:]
    }

    func sendToDevice(deviceID: String, messages: [String: JSONValue]) async throws -> Int {
        let data = try await e2ee("api/e2ee/to_device", method: "POST", body: [
            "device_id": .string(deviceID), "txn_id": .string(UUID().uuidString.lowercased()),
            "type": .string("m.room.encrypted"), "messages": .object(messages)
        ])
        guard case .object(var response) = try JSONDecoder().decode(JSONValue.self, from: data) else { throw ZooError.invalidResponse }
        if case .object(let envelope)? = response["data"] { response = envelope }
        guard let undelivered = response["undelivered"]?.array else { throw ZooError.invalidResponse }
        return undelivered.count
    }

    func saveAccepted(pin: String, signature: SignatureUpload, reauth: String) async throws {
        let pinValue = try JSONDecoder().decode(JSONValue.self, from: Data(pin.utf8))
        _ = try await e2ee("api/e2ee/tofu_pins", method: "POST", body: ["pins": .array([pinValue]), "replace": .bool(true)], reauth: reauth)
        _ = try await e2ee("api/e2ee/cross_signing/signatures", method: "POST", body: ["signatures": .array([.object([
            "target_user_id": .string(signature.targetUserId), "target_key_id": .string(signature.targetKeyId),
            "signer_key_id": .string(signature.signerKeyId), "signature": .string(signature.signature)
        ])])], reauth: reauth)
    }
}

/// 検証が終わったあとの、相手のマスター鍵への署名とピンの保存の状態。
enum SasSaveState: Equatable {
    case idle, saving, saved
    /// 再認証できなかった、または古くなった。再認証し直して保存する。
    case reauth
    case failed
}

enum SasErrorCode: Equatable {
    case needsUnlock, sendFailed, receiveFailed, failed
}

/// 画面に出す検証 1 件。
struct SasEntry: Identifiable, Equatable {
    var view: SasView
    var busy = false
    var save = SasSaveState.idle
    var error: SasErrorCode?

    var id: String { view.transactionId }

    static func == (lhs: SasEntry, rhs: SasEntry) -> Bool {
        lhs.view.transactionId == rhs.view.transactionId && lhs.view.phase == rhs.view.phase && lhs.view.emoji == rhs.view.emoji
            && lhs.busy == rhs.busy && lhs.save == rhs.save && lhs.error == rhs.error
    }
}

/// 他のユーザーの絵文字 SAS 検証（#368、iOS は #685）。Web の `sasController.ts` と同じ流れ（QR は入れない）。
///
/// 検証の状態機械と鍵は zoo-crypto（`SasVerification`）にあり、ここは届いたメッセージを渡し、返ったメッセージを
/// 暗号化して to-device で送る。処理と送信は 1 つずつ順に行う（同じ相手への `mac` と `done` の順を守る）。
@Observable @MainActor
final class E2eeVerification {
    private(set) var trustRevision = 0
    /// 期限（10 分）。これより古い未処理のメッセージは捨てる。
    static let timeoutMs: UInt64 = 10 * 60 * 1000
    /// 処理に送信ユーザーの `keys/query` が要るメッセージ。
    private static let needsKeys: Set<String> = ["m.key.verification.request", "m.key.verification.ready"]

    private(set) var entries: [SasEntry] = []
    /// まだ処理できずに残しているメッセージの数（暗号ストアを使えないとき、鍵を取れないとき）。
    var pendingMessageCount: Int { inbox.count }
    /// 保存できたら呼ぶ（検証済みの表示を読み直す）。
    @ObservationIgnored var onSaved: ((String) -> Void)?
    /// 直前の再認証の ID トークンを取る。
    @ObservationIgnored var reauthenticate: () async throws -> String = { throw ReauthError.browserUnavailable }

    @ObservationIgnored private let store: () -> CryptoStore?
    @ObservationIgnored private let api: any E2eeVerificationAPI
    @ObservationIgnored private let now: () -> UInt64
    @ObservationIgnored private var live: [String: SasVerification] = [:]
    @ObservationIgnored private var inbox: [(message: String, receivedAt: UInt64)] = []
    @ObservationIgnored private var tail: Task<Void, Never>?
    @ObservationIgnored private var saving = Set<String>()
    /// 受け取ったメッセージ（重複を捨てる）。
    @ObservationIgnored private var handled = Set<String>()

    init(store: @escaping () -> CryptoStore?, api: any E2eeVerificationAPI,
         now: @escaping () -> UInt64 = { UInt64(Date().timeIntervalSince1970 * 1000) }) {
        self.store = store
        self.api = api
        self.now = now
    }

    // MARK: - 操作

    /// `userID` に検証を求める。`transaction_id` を返す。
    @discardableResult
    func request(userID: String) async throws -> String {
        try await serial {
            let store = try self.currentStore()
            let keys = try await self.api.queryKeys(userID: userID)
            let started = try store.sasRequest(keysQuery: keys, theirUserID: userID, nowMs: self.now())
            return try await self.apply(started.verification, outgoing: started.outgoing, requireDelivery: true)
        }
    }

    /// to-device で届いた検証のメッセージ（受信時の検証を通った JSON。`E2eeDeviceManager.onVerifications`）。
    func handle(_ messages: [String]) {
        let received = now()
        for message in messages {
            // 同じメッセージが、もう一度届いても（届け先に渡したあとの保存の失敗など）、1 回だけ処理する。
            guard handled.insert(message).inserted else { continue }
            if handled.count > 2000 { handled = [message] }
            inbox.append((message, received))
        }
        Task { @MainActor in try? await self.drain() }
    }

    // MARK: - 相手の検証状態と再検証

    /// 他のユーザーの検証状態。
    enum UserVerificationState: Equatable {
        /// 自分のユーザー署名鍵で署名した（検証済み）。
        case verified
        /// 初めて見たときに信頼した（TOFU。検証はしていない）。
        case trustedOnFirstUse
        /// マスター鍵が変わった、など。再検証するまで、鍵を配らない。
        case needsReverification
        /// 暗号化を設定していない、署名が不正、解錠が要る、など。
        case unavailable
    }

    /// 相手の検証状態を読む。初めて見た相手には TOFU のピンを置く（配布先の判定と同じ。最大 3 回、競合を読み直す）。
    func state(of userID: String) async throws -> UserVerificationState {
        let store = try currentStore()
        for attempt in 0..<3 {
            let keys = try await api.queryKeys(userIDs: [userID, store.userID])
            let distribution = try store.recipients(pages: [keys])
            if distribution.newPins.isEmpty || attempt == 2 {
                guard let trust = distribution.users[userID] else { return .unavailable }
                switch trust.state {
                case .trusted(let basis): return basis == .userSigningSignature || basis == .own ? .verified : .trustedOnFirstUse
                case .needsReverification: return .needsReverification
                default: return .unavailable
                }
            }
            for pin in distribution.newPins { try store.confirmPin(pin, accepted: try await api.registerPin(pin.pin)) }
        }
        return .unavailable
    }

    /// 再検証の確認の画面に出す、相手の今のマスター鍵の公開鍵（指紋）。
    func masterKeyFingerprint(of userID: String) async throws -> String {
        let keys = try await api.queryKeys(userID: userID)
        guard let master = try Self.masterKey(in: keys, user: userID), let fingerprint = Self.fingerprint(master) else { throw ZooError.invalidResponse }
        return fingerprint
    }

    /// 利用者が、確認の画面に出した指紋の鍵を受け入れた（再検証）。署名の直前に鍵を取り直し、同じ鍵のときだけ署名して、
    /// 直前の再認証つきでピンと署名を保存する。違えば `ZooError.invalidResponse`（何も保存しない）。
    func reverify(_ userID: String, expectedFingerprint: String) async throws {
        let store = try currentStore()
        let keys = try await api.queryKeys(userID: userID)
        guard let master = try Self.masterKey(in: keys, user: userID), Self.fingerprint(master) == expectedFingerprint else {
            throw ZooError.invalidResponse
        }
        let token = try await reauthenticate()
        let masterJSON = sasJSONText(master)
        let accepted = try store.acceptMasterKey(userID: userID, masterKey: masterJSON)
        try await api.saveAccepted(pin: accepted.pin, signature: accepted.signature, reauth: token)
        try store.confirmAcceptedMasterKey(userID: userID, masterKey: masterJSON)
        trustRevision += 1
        onSaved?(userID)
    }

    private static func masterKey(in keys: String, user: String) throws -> JSONValue? {
        guard case .object(let root)? = try? JSONDecoder().decode(JSONValue.self, from: Data(keys.utf8)) else { return nil }
        return root["master_keys"]?.object?[user]
    }

    private static func fingerprint(_ master: JSONValue) -> String? {
        guard let values = master.object?["keys"]?.object?.values, values.count == 1 else { return nil }
        return values.first?.string
    }

    func accept(_ id: String) async { await act(id) { try $0.accept() } }

    /// 相手の QR を読み取った。形が正しくない、別の検証の QR は `false`（取りやめずに読み直す）。鍵が違えば取りやめになり、
    /// `true`（結果は画面の状態に出る）。
    func scanQr(_ id: String, bytes: Data) async -> Bool {
        guard let sas = live[id] else { return false }
        patch(id) { $0.busy = true; $0.error = nil }
        defer { patch(id) { $0.busy = false } }
        do {
            let out = try sas.scanQrCode(bytes: bytes)
            _ = try await serial { try await self.apply(sas, outgoing: out) }
            return true
        } catch let failure as E2eeStoreError where failure == .sendNeedsUnlock {
            patch(id) { $0.error = .needsUnlock }
            return true
        } catch {
            return false
        }
    }

    /// 相手がこの端末の QR を読み取ったと、利用者が答えた。
    func confirmQrScanned(_ id: String) async { await act(id) { try $0.confirmQrScanned() } }
    func confirm(_ id: String) async { await act(id) { try $0.confirm() } }
    func startSas(_ id: String) async { await act(id) { try $0.startSas() } }
    func mismatch(_ id: String) async { await act(id) { $0.mismatch() } }
    func cancel(_ id: String) async { await act(id) { $0.cancel() } }

    /// 画面を閉じる。終わっていなければ取りやめてから忘れる。
    func dismiss(_ id: String) async {
        if let entry = entries.first(where: { $0.id == id }), entry.view.phase != .done, entry.view.phase != .cancelled {
            await cancel(id)
        }
        live[id] = nil
        entries.removeAll { $0.id == id }
    }

    /// 再認証し直して、署名とピンを保存し直す。
    func retrySave(_ id: String) { Task { @MainActor in await self.save(id) } }

    /// 10 分を過ぎた検証を取りやめ、残したメッセージを試し直す。
    func expire() async {
        _ = try? await drain()
        let nowMs = now()
        for (id, sas) in live {
            guard live[id] === sas else { continue }
            let out = sas.expire(nowMs: nowMs)
            if !out.isEmpty {
                _ = try? await serial {
                    guard self.live[id] === sas else { return id }
                    return try await self.apply(sas, outgoing: out)
                }
            }
            else { refresh(id, sas) }
        }
    }

    // MARK: - 内部

    private func currentStore() throws -> CryptoStore {
        guard let store = store() else { throw E2eeStoreError.sendNeedsUnlock }
        return store
    }

    private func act(_ id: String, _ step: @escaping (SasVerification) throws -> [VerificationOutgoing]) async {
        guard let sas = live[id] else { return }
        patch(id) { $0.busy = true; $0.error = nil }
        do {
            let out = try step(sas)
            _ = try await serial { try await self.apply(sas, outgoing: out) }
        } catch let failure as E2eeStoreError where failure == .sendNeedsUnlock {
            patch(id) { $0.error = .needsUnlock }
        } catch {
            patch(id) { $0.error = .failed }
        }
        patch(id) { $0.busy = false }
    }

    /// 状態を画面に出し、送るメッセージを送る。検証が終わったら保存を始める。
    @discardableResult
    private func apply(_ sas: SasVerification, outgoing: [VerificationOutgoing], requireDelivery: Bool = false) async throws -> String {
        let view = sas.view()
        live[view.transactionId] = sas
        upsert(view)
        if !outgoing.isEmpty {
            do { try await send(outgoing) } catch {
                patch(view.transactionId) { $0.error = .sendFailed }
                if requireDelivery {
                    live[view.transactionId] = nil
                    entries.removeAll { $0.id == view.transactionId }
                    throw error
                }
            }
        }
        if view.phase == .done, let entry = entries.first(where: { $0.id == view.transactionId }), entry.save == .idle {
            let id = view.transactionId
            Task { @MainActor in await self.save(id) }
        }
        return view.transactionId
    }

    private func refresh(_ id: String, _ sas: SasVerification) { upsert(sas.view()) }

    private func upsert(_ view: SasView) {
        if let index = entries.firstIndex(where: { $0.id == view.transactionId }) {
            entries[index].view = view
        } else {
            entries.append(SasEntry(view: view))
            entries.sort { $0.view.startedAt < $1.view.startedAt }
        }
    }

    private func patch(_ id: String, _ change: (inout SasEntry) -> Void) {
        guard let index = entries.firstIndex(where: { $0.id == id }) else { return }
        change(&entries[index])
    }

    /// 届いたメッセージを、届いた順に渡す。`keys/query` を取れなければ、その検証のメッセージだけを残す。
    private func drain() async throws {
        try await serial {
            var remaining: [(message: String, receivedAt: UInt64)] = []
            let pending = self.inbox
            self.inbox = []
            var held = Set<String>()
            var cache: [String: String] = [:]
            var failed = Set<String>()
            for item in pending {
                guard case .object(let root)? = try? JSONDecoder().decode(JSONValue.self, from: Data(item.message.utf8)),
                      let txn = root["content"]?.object?["transaction_id"]?.string, let type = root["type"]?.string,
                      let sender = root["sender_user_id"]?.string,
                      self.now() &- item.receivedAt <= Self.timeoutMs else { continue }
                if held.contains(txn) { remaining.append(item); continue }
                var keys = "{}"
                if Self.needsKeys.contains(type) {
                    var snapshot = cache[sender]
                    if snapshot == nil, !failed.contains(sender) {
                        do { snapshot = try await self.api.queryKeys(userID: sender); cache[sender] = snapshot }
                        catch { failed.insert(sender) }
                    }
                    guard let snapshot else {
                        held.insert(txn)
                        remaining.append(item)
                        self.patch(txn) { $0.error = .receiveFailed }
                        continue
                    }
                    keys = snapshot
                }
                // 暗号ストアを一時的に使えないとき（ack の失敗で閉じた、開き直している）は、捨てずに残して、あとで試し直す
                // （Olm のメッセージは、2 回は復号できない）。
                guard let store = self.store() else {
                    held.insert(txn)
                    remaining.append(item)
                    continue
                }
                do {
                    if let sas = self.live[txn] {
                        let out = try sas.receive(message: verificationMessageFromJson(stored: item.message), keysQuery: keys)
                        self.patch(txn) { if $0.error == .receiveFailed { $0.error = nil } }
                        _ = try await self.apply(sas, outgoing: out)
                    } else if type == "m.key.verification.request" {
                        let sas = try store.sasFromRequest(message: item.message, keysQuery: keys, nowMs: self.now())
                        _ = try await self.apply(sas, outgoing: [])
                    }
                } catch {
                    // 受け付けないメッセージは、試し直しても通らない。捨てて続ける。
                }
            }
            self.inbox = remaining + self.inbox
        }
    }

    /// 送るメッセージを、順に、1 つずつ、相手の端末ごとに Olm で暗号化して to-device で送る。同じ端末あての複数のメッセージ
    /// （`mac` と `done` など）が、1 つの要求の中で上書きされないよう、メッセージごとに別の要求にする。
    private func send(_ outgoing: [VerificationOutgoing]) async throws {
        for out in outgoing { try await sendOne(out) }
    }

    private func sendOne(_ out: VerificationOutgoing) async throws {
        let store = try currentStore()
        var undelivered = 0
        var messages: [String: JSONValue] = [:]
        var claimed: [String: [String: JSONValue]] = [:]
        for round in 0..<2 {
            messages = [:]
            var claim: [String: JSONValue] = [:]
            for device in out.devices {
                let user = device.userId(), id = device.deviceId()
                let single = claimed[user]?[id]?.object.flatMap { $0.count == 1 ? $0.values.first : nil }
                guard let content = try store.encryptVerification(device, eventType: out.eventType, content: out.content,
                                                                   claimed: single.map(sasJSONText)) else {
                    var perUser = claim[user]?.object ?? [:]
                    perUser[id] = .string("signed_curve25519")
                    claim[user] = .object(perUser)
                    continue
                }
                var perUser = messages[user]?.object ?? [:]
                perUser[id] = try JSONDecoder().decode(JSONValue.self, from: Data(content.utf8))
                messages[user] = .object(perUser)
            }
            if claim.isEmpty || round == 1 {
                if !claim.isEmpty { undelivered += claim.values.reduce(0) { $0 + ($1.object?.count ?? 0) } }
                break
            }
            for (user, devices) in try await api.claim(claim) { claimed[user] = devices.object ?? [:] }
        }
        if !messages.isEmpty { undelivered += try await api.sendToDevice(deviceID: store.deviceID, messages: messages) }
        if undelivered > 0 { throw ZooError.invalidResponse }
    }

    /// 検証が終わった相手のマスター鍵に署名し、ピンと署名を保存する。
    private func save(_ id: String) async {
        guard let sas = live[id], let entry = entries.first(where: { $0.id == id }), entry.view.phase == .done,
              entry.save != .saving, saving.insert(id).inserted else { return }
        defer { saving.remove(id) }
        patch(id) { $0.save = .saving }
        let theirUser = entry.view.theirUserId
        do {
            let token: String
            do { token = try await reauthenticate() } catch { patch(id) { $0.save = .reauth }; return }
            let store = try currentStore()
            let keys = try await api.queryKeys(userID: theirUser)
            guard case .object(let root)? = try? JSONDecoder().decode(JSONValue.self, from: Data(keys.utf8)),
                  let master = root["master_keys"]?.object?[theirUser] else { throw ZooError.invalidResponse }
            let masterJSON = sasJSONText(master)
            let accepted = try store.signSasMasterKey(sas, masterKey: masterJSON)
            try await api.saveAccepted(pin: accepted.pin, signature: accepted.signature, reauth: token)
            try store.confirmAcceptedMasterKey(userID: theirUser, masterKey: masterJSON)
            patch(id) { $0.save = .saved }
            trustRevision += 1
            onSaved?(theirUser)
        } catch let failure as E2eeAPIError where failure.isReauthRequired {
            patch(id) { $0.save = .reauth }
        } catch {
            patch(id) { $0.save = .failed }
        }
    }

    /// 操作を 1 つずつ順に行う。
    private func serial<T: Sendable>(_ operation: @escaping @MainActor () async throws -> T) async throws -> T {
        let previous = tail
        let task = Task { @MainActor in
            await previous?.value
            return try await operation()
        }
        tail = Task { _ = try? await task.value }
        return try await task.value
    }
}
