import Foundation
import Observation

/// 自分の端末あての to-device メッセージの Shape（`api/shapes/e2ee_to_device?device_id=`）の行。
/// Shape は bigint の列を文字列で配り、jsonb の列は JSON か文字列で届くので、`content` は文字列なら JSON として読み直す。
struct ToDeviceShapeRow: Decodable, Sendable {
    let id: String
    let sender_user_id: String
    let type: String
    let content: JSONValue

    var row: CryptoStore.ToDeviceRow {
        var json = content
        // 文字列で届いた jsonb を、JSON として読み直す（読めなければ null にして、復号できないメッセージとして捨てる）。
        if case .string(let text) = content { json = (try? JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))) ?? .null }
        return CryptoStore.ToDeviceRow(id: id, senderUserID: sender_user_id, type: type,
                                       content: String(decoding: (try? JSONEncoder().encode(json)) ?? Data("null".utf8), as: UTF8.self))
    }

    /// 残っている行を、ID の昇順（桁数、文字列の順）にする。サーバーは ack で行を消すので、Shape には未処理のものだけが残る。
    static func ordered(_ rows: [ToDeviceShapeRow]) -> [CryptoStore.ToDeviceRow] {
        rows.map(\.row).sorted { $0.id.count == $1.id.count ? $0.id < $1.id : $0.id.count < $1.id.count }
    }
}

/// ワンタイム鍵の数（`keys/upload` の応答）。
struct KeyCounts: Equatable {
    var oneTimeKeys = 0
    var hasUnusedFallbackKey = false

    init(oneTimeKeys: Int = 0, hasUnusedFallbackKey: Bool = false) {
        self.oneTimeKeys = oneTimeKeys
        self.hasUnusedFallbackKey = hasUnusedFallbackKey
    }

    /// 数が整数で表せない、または負のときは、不正な応答として投げる（`Int(_:)` で停止しない）。
    init(_ response: JSONValue) throws {
        if case .object(let object) = response {
            if case .object(let counts)? = object["one_time_key_counts"], case .number(let count)? = counts["signed_curve25519"] {
                guard let value = Int(exactly: count), value >= 0 else { throw ZooError.invalidResponse }
                oneTimeKeys = value
            }
            if case .array(let types)? = object["unused_fallback_key_types"] { hasUnusedFallbackKey = types.contains(.string("signed_curve25519")) }
        }
    }
}

/// 端末の登録のあとに使う E2EE の API。
@MainActor
protocol E2eeKeysAPI {
    /// `POST /api/e2ee/keys/upload`。鍵が空なら、今の数を読むだけになる。
    func uploadKeys(deviceID: String, oneTimeKeys: [String: JSONValue], fallbackKeys: [String: JSONValue]) async throws -> KeyCounts
    /// 送信ユーザーと自分の `keys/query`（50 人ずつ）。端末鍵、クロス署名鍵、署名、ピンをまとめた JSON。
    func queryKeys(userIDs: [String]) async throws -> String
    /// `POST /api/e2ee/to_device/ack`（1000 件ずつ）。
    func ackToDevice(deviceID: String, ids: [String]) async throws
}

extension APIClient: E2eeKeysAPI {
    func uploadKeys(deviceID: String, oneTimeKeys: [String: JSONValue], fallbackKeys: [String: JSONValue]) async throws -> KeyCounts {
        try KeyCounts(try await e2eeJSON("api/e2ee/keys/upload", method: "POST", body: [
            "device_id": .string(deviceID), "one_time_keys": .object(oneTimeKeys), "fallback_keys": .object(fallbackKeys)
        ]))
    }

    func queryKeys(userIDs: [String]) async throws -> String {
        // サーバーの上限は 100 人。TUI・Android と同じく 50 人ずつにする。
        var merged: [String: JSONValue] = [:]
        var signatures: [JSONValue] = []
        var snapshot: [String: JSONValue] = [:]
        var start = 0
        while start < userIDs.count {
            let chunk = Array(userIDs[start..<min(start + 50, userIDs.count)])
            start += 50
            guard case .object(let response) = try await e2eeJSON("api/e2ee/keys/query", method: "POST",
                                                                  body: ["user_ids": .array(chunk.map(JSONValue.string))]) else { continue }
            for field in ["master_keys", "self_signing_keys", "user_signing_keys", "tofu_pins"] {
                var previous: [String: JSONValue] = [:]
                if case .object(let existing)? = snapshot[field] { previous = existing }
                if case .object(let added)? = response[field] { previous.merge(added) { _, new in new } }
                snapshot[field] = .object(previous)
            }
            if case .object(let devices)? = response["device_keys"] { merged.merge(devices) { _, new in new } }
            if case .array(let rows)? = response["signatures"] { signatures += rows }
        }
        snapshot["device_keys"] = .object(merged)
        snapshot["signatures"] = .array(signatures)
        var seen = Set<String>()
        snapshot["user_ids"] = .array(userIDs.filter { seen.insert($0).inserted }.map(JSONValue.string))
        return String(decoding: try JSONEncoder().encode(JSONValue.object(snapshot)), as: UTF8.self)
    }

    func ackToDevice(deviceID: String, ids: [String]) async throws {
        var start = 0
        while start < ids.count {
            let chunk = Array(ids[start..<min(start + 1000, ids.count)])
            start += 1000
            _ = try await e2eeJSON("api/e2ee/to_device/ack", method: "POST",
                                   body: ["device_id": .string(deviceID), "ids": .array(chunk.map(JSONValue.string))])
        }
    }
}

/// ログイン後の端末の登録と、鍵の補充、to-device の処理（#685。React の `deviceManager.ts`、Android の
/// `E2eeDeviceManager` と同じ流れ）。
///
/// 1. 暗号ストアを開く。端末がなければ作る。
/// 2. 登録していなければ `POST /api/e2ee/devices` で登録する（応答をなくしても同じ鍵で送り直す）。
/// 3. サーバーのワンタイム鍵が最大数に足りなければ作って送る。フォールバック鍵が使われたら作り直す。
/// 4. to-device の Shape を購読し、処理したメッセージを ack で消す。Olm セッションを新しく作った
///    （ワンタイム鍵を使われた）ら、3 をもう一度行う。
///
/// 失敗は表示せず、暗号ストアを閉じてから、間隔をのばしながら（5 秒から 5 分まで）初めからやり直す
/// （開き直すので、保存より先に進んだメモリーの状態を使い続けない）。
///
/// 端末が別の端末から削除されたとき：登録の 409 `E2EE_DEVICE_EXISTS`、鍵のアップロードや ack の 404
/// `E2EE_DEVICE_NOT_FOUND` を候補にし、`GET /api/e2ee/devices` の一覧に今の端末がないと確かめられたときだけ、
/// 暗号ストアを消さずに端末だけを作り直して、新しい端末として登録する。
@Observable @MainActor
final class E2eeDeviceManager {
    /// 登録を終えて使える暗号ストア（初回設定と解錠の画面、送受信と共有する）。止まるときに `nil` に戻す。
    private(set) var readyStore: CryptoStore?

    private let api: any E2eeDeviceAPI & E2eeKeysAPI
    private let files: CryptoStoreFiles
    private let userID: String
    private let displayName: String
    private let observeToDevice: (String, @escaping @MainActor ([CryptoStore.ToDeviceRow]) -> Void) async throws -> Void
    private let firstRetryDelay: Duration
    private let maxRetryDelay: Duration
    @ObservationIgnored private let onError: (any Error) -> Void
    /// 登録と補充を終えて、暗号ストアを使えるようになったとき。
    @ObservationIgnored var onReady: (() -> Void)?
    /// to-device で Megolm セッションの鍵を受け取ったとき（読めなかったメッセージを読めるようになる）。
    @ObservationIgnored var onKeysReceived: (() -> Void)?
    /// 受信時の検証を通った、SAS の検証のメッセージ（JSON）が届いた。
    @ObservationIgnored var onVerifications: (([String]) -> Void)?
    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored private var store: CryptoStore?

    init(api: any E2eeDeviceAPI & E2eeKeysAPI, files: CryptoStoreFiles, userID: String,
         displayName: String = DeviceRegistration.defaultDisplayName(),
         observeToDevice: @escaping (String, @escaping @MainActor ([CryptoStore.ToDeviceRow]) -> Void) async throws -> Void,
         firstRetryDelay: Duration = .seconds(5), maxRetryDelay: Duration = .seconds(300),
         onError: @escaping (any Error) -> Void = { _ in }) {
        self.api = api
        self.files = files
        self.userID = userID
        self.displayName = displayName
        self.observeToDevice = observeToDevice
        self.firstRetryDelay = firstRetryDelay
        self.maxRetryDelay = maxRetryDelay
        self.onError = onError
    }

    /// 処理を始める。動いていれば何もしない。
    private var started = false

    func start() {
        guard loop == nil else { return }
        started = true
        loop = Task { await self.runLoop() }
    }

    /// 処理を止めて、暗号ストアを閉じる。
    /// 止め終えるまで `loop` を持ち続けるので、そのあいだの `start()` は何もしない（新しい処理が、古い処理の後片付けに
    /// 巻き込まれない）。
    func stop() async {
        guard let running = loop else { return }
        started = false
        running.cancel()
        await running.value
        // 止めている間に別の `stop()` が終えて、新しい処理が始まっていたら、その処理の暗号ストアには触れない。
        guard loop == running else { return }
        loop = nil
        readyStore = nil
        store?.close()
        store = nil
    }

    /// 登録を終えて使える暗号ストアを待つ（初回設定と解錠が使う）。取り消されたら投げる。
    func waitForStore() async throws -> CryptoStore {
        while true {
            if let readyStore { return readyStore }
            // 止めたあとは待ち続けない（待っている鍵バックアップなども終わらせる）。
            if !started { throw CancellationError() }
            try await Task.sleep(for: .milliseconds(50))
        }
    }

    private struct DeviceGone: Error { let deviceID: String; let failure: E2eeAPIError }

    private func runLoop() async {
        var backoff = firstRetryDelay
        while !Task.isCancelled {
            var connected = false
            // 親の処理が止められていないのに購読が `CancellationError` を返したときも、後片付けとバックオフを通して
            // やり直す（止められたときだけ抜ける）。
            do { try await runOnce { connected = true } } catch {
                if !Task.isCancelled { onError(error) }
            }
            readyStore = nil
            store?.close()
            store = nil
            if Task.isCancelled { break }
            // 登録と補充を終えて Shape を受け取れていたら、そのあとの切断は連続した失敗ではないので、
            // 最初の間隔から数え直す。続けて失敗したら倍にする。
            let wait = connected ? firstRetryDelay : backoff
            backoff = min(wait * 2, maxRetryDelay)
            do { try await Task.sleep(for: wait) } catch { break }
        }
    }

    private func runOnce(connected: () -> Void) async throws {
        let registration = DeviceRegistration(api: api, files: files, userID: userID, displayName: displayName)
        var current = try await registration.ensureRegistered()
        store = current
        var replaced = false
        while true {
            do {
                try await serve(current, connected: connected)
            } catch let candidate as DeviceGone {
                readyStore = nil
                // 作り直した端末がすぐにまた候補になったら、間隔をあけて試し直す（同じ回で作り直し続けない）。
                if replaced { throw candidate.failure }
                // 誤った応答や一時的な失敗で暗号ストアの記録を失わないよう、一覧で確かめ直す。
                guard let list = try? await api.deviceIDs(), !list.contains(candidate.deviceID) else { throw candidate.failure }
                replaced = true
                try current.replaceDevice()
                try await registration.register(current)
            }
        }
    }

    /// 補充をして、to-device を処理する。終わらない（失敗で抜ける）。
    private func serve(_ store: CryptoStore, connected: () -> Void) async throws {
        let deviceID = store.deviceID
        // 前の実行で、処理済みにしたが、届け先に渡せなかった検証のメッセージ（保存したもの）を、先に届ける。
        deliverVerifications(store)
        try await replenish(store, counts: try await uploadKeys(deviceID, [:], [:]))
        readyStore = store
        onReady?()
        // 行の到着（同期の通知）と処理（通信を待つ）を分ける。最新の一覧だけを残す。
        let (rows, continuation) = AsyncStream<[CryptoStore.ToDeviceRow]>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let observer = Task { @MainActor in
            do {
                try await self.observeToDevice(deviceID) { continuation.yield($0) }
            } catch {
                continuation.finish()
                throw error
            }
            continuation.finish()
            throw ZooError.invalidResponse
        }
        defer { observer.cancel() }
        // ack を送れた行は、Shape から消えるまで処理し直さない（ack より先に次の一覧が届くため）。
        var acknowledged = Set<String>()
        for await list in rows {
            connected()
            acknowledged.formIntersection(list.map(\.id))
            let waiting = list.filter { !acknowledged.contains($0.id) }
            if !waiting.isEmpty {
                let result = try await process(store, deviceID: deviceID, rows: waiting)
                acknowledged.formUnion(result.processed)
            }
        }
        // 止められたときは、購読の終了を待たずに抜ける（defer が購読を取り消す）。
        try Task.checkCancellation()
        try await observer.value
    }

    private func uploadKeys(_ deviceID: String, _ oneTimeKeys: [String: JSONValue], _ fallbackKeys: [String: JSONValue]) async throws -> KeyCounts {
        do { return try await api.uploadKeys(deviceID: deviceID, oneTimeKeys: oneTimeKeys, fallbackKeys: fallbackKeys) }
        catch let failure as E2eeAPIError where failure.isDeviceNotFound { throw DeviceGone(deviceID: deviceID, failure: failure) }
    }

    private func replenish(_ store: CryptoStore, counts: KeyCounts) async throws {
        guard let upload = try store.prepareKeyUpload(oneTimeKeyCount: counts.oneTimeKeys, hasUnusedFallbackKey: counts.hasUnusedFallbackKey) else { return }
        _ = try await uploadKeys(store.deviceID, upload.oneTimeKeys, upload.fallbackKeys)
        try store.markKeysPublished()
    }

    /// 保存してある、届け先に渡していない検証のメッセージを届け、届けたあとに一覧から外す（届ける前には消さない）。外す保存が
    /// 失敗したら、メッセージはそのまま残り、あとでもう一度届く（届け先が重複を捨てる）。
    private func deliverVerifications(_ store: CryptoStore) {
        let pending = store.pendingVerifications()
        guard !pending.isEmpty else { return }
        onVerifications?(pending)
        try? store.discardVerifications(pending)
    }

    /// 行を処理して ack を送り、結果を返す。
    private func process(_ store: CryptoStore, deviceID: String, rows: [CryptoStore.ToDeviceRow]) async throws -> CryptoStore.ToDeviceResult {
        var senders = Set(rows.map(\.senderUserID))
        senders.insert(userID)
        let keys = try await api.queryKeys(userIDs: senders.sorted())
        let result: CryptoStore.ToDeviceResult
        do { result = try store.processToDevice(rows, keysQuery: keys) }
        catch {
            // 途中の行で保存に失敗しても、すでに処理済みにした行の検証メッセージは、再試行では復号できないので、ここで届ける
            // （保存したものは、開き直したあとにも取り出せる）。
            deliverVerifications(store)
            // 失敗の前の行で鍵を保存していても、再試行ではその行を処理しないので、復号のキャッシュは、ここで捨てさせる。
            onKeysReceived?()
            throw error
        }
        // 処理済みの印は、もう保存した。この先の ack や鍵の補充が通信で失敗しても、再試行では復号を省くので、鍵と検証のメッセージは、
        // ここで届ける（ack より前。届け先が覚えておく）。
        if result.roomKeys > 0 { onKeysReceived?() }
        deliverVerifications(store)
        if !result.processed.isEmpty {
            do { try await api.ackToDevice(deviceID: deviceID, ids: result.processed) }
            catch let failure as E2eeAPIError where failure.isDeviceNotFound { throw DeviceGone(deviceID: deviceID, failure: failure) }
            try store.acknowledged(result.processed)
        }
        if result.newOlmSessions > 0 { try await replenish(store, counts: try await uploadKeys(deviceID, [:], [:])) }
        return result
    }
}
