import Foundation
import ZooCrypto

/// 暗号化チャンネルの表示に使う文言（Web の `ja.json` の `e2ee.*` と同じ）。
enum E2eeText {
    static let keyMissing = "このメッセージを復号する鍵がまだ届いていません。"
    static let locked = "この端末で暗号化を解錠すると、このメッセージを読めます。"
    /// 復号や照合に失敗した行。本文はないが、「表示しない」とは言わず、注意として知らせる（アイコンと文だけ）。
    static let invalid = "このメッセージは検証に失敗しました。"
    static let senderUnverified = "未検証の送信者"
    static let senderUnauthenticated = "送信者を確かめられません"
    static let revisionStale = "サーバーが古い版を返しました。最新の版を表示しています"
    static let revisionConflict = "同じ版で内容の違うメッセージが届きました。先に届いた方を表示しています"
    static let deviceNotReady = "この端末では、まだ暗号化の準備ができていません。暗号化の設定と解錠を確かめてください。"
    static let sendNeedsUnlock = "暗号化チャンネルに送るには、この端末で暗号化を解錠してください。"
    static let sendStaleEpoch = "メンバーが変わり続けたため、送信できませんでした。もう一度送ってください。"
    static let editConflict = "このメッセージはほかの場所で編集されました。読み込み直してから編集してください。"
    static let unencrypted = "このメッセージは暗号化されていないため表示しません。"
    /// 暗号化したメッセージより後ろにある、暗号化されていないメッセージ（正しいサーバーは作れない）。
    /// 暗号化に切り替える前と後の境目（Web の `e2ee.legacy.divider`）。
    static let encryptionStarts = "ここから暗号化されています"
    static let unencryptedBadge = "暗号化されていません"
    /// ⓘ をタップしたときに出す、くわしい説明。
    static let unencryptedBadgeHelp = "暗号化したメッセージより後ろにある、暗号化されていないメッセージです。暗号化したチャンネルでは、正しいサーバーはこのようなメッセージを作れません。サーバーが足したか、書き換えたおそれがあります。"
    static let editForbidden = "暗号化されていないメッセージは、暗号化チャンネルでは編集できません。"
}

/// 暗号化チャンネルの送受信の誤り。`description` は利用者に見せる文言。
enum MessageEncryptionError: Error, Equatable, LocalizedError {
    case deviceNotReady, sendNeedsUnlock, sendStaleEpoch, editConflict, editForbidden
    case undeliverable(String)
    /// 暗号化を求めたのに、サーバーが暗号化しない会話にした（格下げ）。
    case downgraded
    /// 鍵を配れない参加者がいる（理由つき。名前は、画面が付ける）。
    case undeliverableUsers([MessageEncryption.Undeliverable])

    var errorDescription: String? {
        switch self {
        case .deviceNotReady: E2eeText.deviceNotReady
        case .sendNeedsUnlock: E2eeText.sendNeedsUnlock
        case .sendStaleEpoch: E2eeText.sendStaleEpoch
        case .editConflict: E2eeText.editConflict
        case .editForbidden: E2eeText.editForbidden
        case .undeliverable(let reason): reason
        case .undeliverableUsers: "鍵を配れない参加者がいます。"
        case .downgraded: "暗号化を求めましたが、暗号化されない会話になりました。安全のため、この会話は使わないでください。"
        }
    }
}

/// 復号した本文と、表示に添える注意書き。本文が `nil` なら `notice` だけを出す。
struct DecryptedBody: Equatable {
    var body: String?
    var notice: String?
    var revision: MessageRevision = .latest
    var payload: MessagePayload? = nil

    static func == (lhs: DecryptedBody, rhs: DecryptedBody) -> Bool {
        lhs.body == rhs.body && lhs.notice == rhs.notice && lhs.revision == rhs.revision
    }
}

enum MessageRevision: Equatable { case latest, stale, conflict }

/// 暗号化チャンネルのメッセージ 1 件の、復号と編集に使う列（サーバーの行）。
struct EncryptedRow: Equatable {
    let channelID: String
    let id: String
    let parentID: String?
    let userID: String
    let kind: String
    let epoch: UInt64
    let revision: UInt64
    let ciphertext: String
}

/// E2EE の API を呼ぶ。応答は `{"data": ...}` の封筒を外した JSON。
@MainActor
protocol E2eeRequesting {
    func e2eeJSON(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?) async throws -> JSONValue
}

extension E2eeRequesting {
    func e2eeJSON(_ path: String, method: String, body: [String: JSONValue]?) async throws -> JSONValue {
        try await e2eeJSON(path, method: method, query: [:], body: body)
    }

    /// 平文・暗号文とも、参加者の一覧を取らずに再参加の条件を確かめる。
    func rejoinThread(messageID: String) async throws -> Bool {
        guard UUID(uuidString: messageID) != nil else { throw ZooError.invalidResponse }
        let path = "api/threads/\(messageID)", lean = ["participants": "none"]
        let value = try await e2eeJSON(path, method: "GET", query: lean, body: nil)
        let status = try JSONDecoder().decode(ThreadMembership.self, from: JSONEncoder().encode(value))
        guard status.can_participate else { return false }
        if status.thread_archive_at != nil { _ = try await e2eeJSON("\(path)/unarchive", method: "POST", query: lean, body: nil) }
        if status.membership != "joined" { _ = try await e2eeJSON("\(path)/join", method: "POST", query: lean, body: nil) }
        return true
    }
}

extension APIClient: E2eeRequesting {
    func e2eeJSON(_ path: String, method: String, query: [String: String], body: [String: JSONValue]?) async throws -> JSONValue {
        let data = try await e2ee(path, method: method, query: query, body: body)
        guard let value = try? JSONDecoder().decode(JSONValue.self, from: data) else { return .object([:]) }
        if case .object(let object) = value, let inner = object["data"] { return inner }
        return value
    }
}

extension JSONValue {
    var object: [String: JSONValue]? { if case .object(let value) = self { value } else { nil } }
    var string: String? { if case .string(let value) = self { value } else { nil } }
    var array: [JSONValue]? { if case .array(let value) = self { value } else { nil } }
    var uint: UInt64? { if case .number(let value) = self { UInt64(exactly: value) } else { nil } }
}

private func jsonText(_ value: JSONValue) -> String { String(decoding: try! JSONEncoder().encode(value), as: UTF8.self) }

/// 暗号化チャンネルの送受信（docs/e2ee.md の「暗号化チャンネルと DM の送受信」。#361）。Android の
/// `AndroidMessageEncryption`、Web と同じ recipients・TOFU・Olm/Megolm を使い、暗号文だけをサーバーに渡す。
///
/// 送信は 1 つずつ。配布先は `membership_epoch` で確かめ、`E2EE_STALE_EPOCH`（409）のときは、そのセッションを捨てて
/// 新しい epoch でセッションを作り直す（3 回まで）。配れなかった鍵のセッションで本文を送らない。
@MainActor
final class MessageEncryption {
    private let store: () -> CryptoStore?
    private let api: any E2eeRequesting
    private let now: () -> UInt64
    private let makeID: () -> String
    private let uploadBackup: () async -> Void

    private final class SendingSession {
        let owner: CryptoStore
        let session: OutboundSession
        /// 作ったときの送信端末の Curve25519 鍵。端末を作り直すと変わり、そのセッションは使えない。
        let senderKey: String
        var shared = Set<String>()
        init(owner: CryptoStore, session: OutboundSession) {
            self.owner = owner
            self.session = session
            senderKey = owner.curve25519Key
        }
    }

    private var outbound: [String: SendingSession] = [:]
    private var latestEpochs: [String: UInt64] = [:]
    private var tail: Task<Void, Never>?

    init(store: @escaping () -> CryptoStore?, api: any E2eeRequesting,
         now: @escaping () -> UInt64 = { UInt64(Date().timeIntervalSince1970 * 1000) },
         makeID: @escaping () -> String = { UUID().uuidString.lowercased() },
         uploadBackup: @escaping () async -> Void = {}) {
        self.store = store
        self.api = api
        self.now = now
        self.makeID = makeID
        self.uploadBackup = uploadBackup
    }

    private func currentStore() throws -> CryptoStore {
        guard let store = store() else { throw MessageEncryptionError.deviceNotReady }
        return store
    }

    private func request(_ path: String, method: String = "GET", query: [String: String] = [:],
                         body: [String: JSONValue]? = nil) async throws -> [String: JSONValue] {
        try await api.e2eeJSON(path, method: method, query: query, body: body).object ?? [:]
    }

    private func deviceKey(_ device: VerifiedDeviceKeys) -> String {
        "\(device.userId())|\(device.deviceId())|\(device.curve25519Key())"
    }

    private func discard(_ channel: String) { outbound.removeValue(forKey: channel) }

    /// 配布先のページを、同じ epoch で最後まで読む。途中で epoch が変わったら（または古ければ）最初から読み直す。
    private func pages(_ channel: String, minimum: UInt64) async throws -> [[String: JSONValue]] {
        let encodedChannel = channel.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? channel
        for _ in 0..<5 {
            var pages: [[String: JSONValue]] = []
            var epoch: String?
            var after: String?
            do {
                repeat {
                    var query: [String: String] = [:]
                    if let epoch { query["epoch"] = epoch }
                    if let after { query["after"] = after }
                    let page = try await request("api/e2ee/channels/\(encodedChannel)/recipients", query: query)
                    guard let current = page["membership_epoch"]?.uint else { throw ZooError.invalidResponse }
                    if current < minimum || (epoch != nil && epoch != String(current)) { break }
                    epoch = String(current)
                    pages.append(page)
                    guard pages.count <= 200 else { throw MessageEncryptionError.undeliverable("配布先のページ数が上限を超えました") }
                    let previous = after
                    switch page["next_after"] {
                    case nil, .null: after = nil
                    case .string(let value): after = value
                    default: throw ZooError.invalidResponse
                    }
                    guard after == nil || after != previous else { throw MessageEncryptionError.undeliverable("配布先のページが進みませんでした") }
                } while after != nil
                if let epoch, let value = UInt64(epoch), value >= minimum, after == nil { return pages }
            } catch let failure as E2eeAPIError {
                if failure.code != "E2EE_STALE_EPOCH" { throw failure }
            }
        }
        throw MessageEncryptionError.sendStaleEpoch
    }

    /// 鍵を配れない相手と、その理由（暗号化の設定がない、鍵が不正、再検証が必要、など）。
    struct Undeliverable: Equatable, Sendable {
        enum Reason: String, Equatable, Sendable { case notSetUp, invalidKeys, needsReverification, needsUnlock, pinPending, blocked, warning }
        let userID: String
        let reason: Reason
    }

    /// 送信欄の上に出す、配布先の要約（docs/e2ee.md の「送信画面の表示」）。
    struct RecipientSummary: Equatable, Sendable {
        /// 前回の送信から、配布先に増えたユーザー。
        var added: [String] = []
        /// 鍵を配れない相手と理由。
        var undeliverable: [Undeliverable] = []
        var isEmpty: Bool { added.isEmpty && undeliverable.isEmpty }
    }

    /// 配布先の判定。新しい TOFU のピンは、サーバーに登録してから確定する（競合したら読み直す）。
    private func evaluate(_ store: CryptoStore, channel: String, minimum: UInt64) async throws -> (epoch: UInt64, distribution: Distribution) {
        var pages = try await pages(channel, minimum: minimum)
        for _ in 0..<5 {
            let distribution: Distribution
            do { distribution = try store.recipients(pages: pages.map { jsonText(.object($0)) }) }
            catch E2eeStoreError.sendNeedsUnlock { throw MessageEncryptionError.sendNeedsUnlock }
            if distribution.newPins.isEmpty {
                guard let epoch = pages.first?["membership_epoch"]?.uint else { throw ZooError.invalidResponse }
                return (epoch, distribution)
            }
            var conflict = false
            for pin in distribution.newPins {
                var accepted = true
                do {
                    let value = try JSONDecoder().decode(JSONValue.self, from: Data(pin.pin.utf8))
                    _ = try await request("api/e2ee/tofu_pins", method: "POST", body: ["pins": .array([value]), "replace": .bool(false)])
                } catch let failure as E2eeAPIError {
                    if failure.code != "E2EE_PIN_CONFLICT" { throw failure }
                    conflict = true
                    accepted = false
                }
                try store.confirmPin(pin, accepted: accepted)
            }
            if conflict { pages = try await self.pages(channel, minimum: minimum) }
        }
        throw MessageEncryptionError.undeliverable("TOFU のピンを確定できませんでした")
    }

    /// 鍵を配れない相手と理由。配れない相手がいなければ空。
    private func undeliverable(_ store: CryptoStore, _ distribution: Distribution) -> [Undeliverable] {
        var result: [Undeliverable] = []
        for (user, trust) in distribution.users.sorted(by: { $0.key < $1.key }) {
            switch trust.state {
            case .trusted:
                // 信頼できても、鍵を配れる端末がなければ、配れない（暗号化の設定をしていない）。
                if user != store.userID, !distribution.devices.contains(where: { $0.userId() == user }) {
                    result.append(Undeliverable(userID: user, reason: .notSetUp))
                }
            case .noCrossSigningKeys: result.append(Undeliverable(userID: user, reason: .notSetUp))
            case .invalidCrossSigningKeys: result.append(Undeliverable(userID: user, reason: .invalidKeys))
            case .needsReverification: result.append(Undeliverable(userID: user, reason: .needsReverification))
            case .needsUnlock: result.append(Undeliverable(userID: user, reason: .needsUnlock))
            case .pinPending: result.append(Undeliverable(userID: user, reason: .pinPending))
            case .blocked: result.append(Undeliverable(userID: user, reason: .blocked))
            }
        }
        // 利用者に関わる警告（自分のマスター鍵の変更など）は、ユーザーに結び付かないので、自分の分として出す。
        if !distribution.warnings.isEmpty, !result.contains(where: { $0.userID == store.userID }) {
            result.append(Undeliverable(userID: store.userID, reason: .warning))
        }
        return result
    }

    /// 送る前の要約。前回の送信から増えた配布先と、鍵を配れない相手（送信はしない。ピンの確定と記録の保存だけ行う）。
    func summary(channel: String) async throws -> RecipientSummary {
        let store = try currentStore()
        let (epoch, distribution) = try await evaluate(store, channel: channel, minimum: latestEpochs[channel] ?? 0)
        latestEpochs[channel] = max(latestEpochs[channel] ?? 0, epoch)
        return summary(store, channel: channel, distribution: distribution)
    }

    private func summary(_ store: CryptoStore, channel: String, distribution: Distribution) -> RecipientSummary {
        let current = Set(distribution.users.keys).subtracting([store.userID])
        let previous = Set(store.recordedMembers(channel: channel) ?? [])
        // 記録がなければ（初めて）、増えたとは言わない。
        let added = store.recordedMembers(channel: channel) == nil ? [] : current.subtracting(previous).sorted()
        return RecipientSummary(added: added, undeliverable: undeliverable(store, distribution))
    }

    /// 自分の鍵を信頼できない場合は止め、受信者の未設定などは既存の配布先の要約に表示する。
    private func recipients(_ store: CryptoStore, channel: String, minimum: UInt64,
                            onRecipients: ((RecipientSummary) -> Void)?) async throws -> (UInt64, [VerifiedDeviceKeys]) {
        let (epoch, distribution) = try await evaluate(store, channel: channel, minimum: minimum)
        let summary = summary(store, channel: channel, distribution: distribution)
        onRecipients?(summary)
        let blocked = summary.undeliverable
        let ownFailures = blocked.filter { $0.userID == store.userID }
        guard ownFailures.isEmpty else { throw MessageEncryptionError.undeliverableUsers(ownFailures) }
        // 送れるので、配布先を記録する（次の「前回から増えた配布先」の基準）。
        try? store.recordMembers(channel: channel, users: Array(Set(distribution.users.keys).subtracting([store.userID])))
        return (epoch, distribution.devices)
    }

    /// セッションの鍵を、受信端末に Olm で配る。配れなかった端末があれば投げる。
    private func share(_ store: CryptoStore, outbound: OutboundSession, devices: [VerifiedDeviceKeys]) async throws {
        var start = 0
        while start < devices.count {
            let batch = Array(devices[start..<min(start + 100, devices.count)])
            start += 100
            let needs = batch.filter { store.needsOlm($0) }
            var claim: [String: JSONValue] = [:]
            for device in needs {
                var perUser = claim[device.userId()]?.object ?? [:]
                perUser[device.deviceId()] = .string("signed_curve25519")
                claim[device.userId()] = .object(perUser)
            }
            let claimed = needs.isEmpty ? [:] : (try await request("api/e2ee/keys/claim", method: "POST",
                                                                   body: ["one_time_keys": .object(claim)]))["one_time_keys"]?.object ?? [:]
            var messages: [String: JSONValue] = [:]
            for device in batch {
                // 払い出されたワンタイム鍵は、1 端末につきちょうど 1 つ。
                let single = claimed[device.userId()]?.object?[device.deviceId()]?.object.flatMap { $0.count == 1 ? $0.values.first : nil }
                guard let encrypted = try store.shareKey(device, outbound: outbound, claimed: single.map(jsonText)) else {
                    throw MessageEncryptionError.undeliverable("受信端末の鍵を取得できませんでした")
                }
                var perUser = messages[device.userId()]?.object ?? [:]
                perUser[device.deviceId()] = try JSONDecoder().decode(JSONValue.self, from: Data(encrypted.utf8))
                messages[device.userId()] = .object(perUser)
            }
            let sent = try await request("api/e2ee/to_device", method: "POST", body: [
                "device_id": .string(store.deviceID), "txn_id": .string(makeID()),
                "type": .string("m.room.encrypted"), "messages": .object(messages)
            ])
            // 応答は `{"undelivered": [...]}`（なければ空の配列）。読めない応答は、配れたと確かめられないので、本文を送らない。
            guard let undelivered = sent["undelivered"]?.array else { throw MessageEncryptionError.undeliverable("鍵の配布の結果を確かめられませんでした") }
            guard undelivered.isEmpty else { throw MessageEncryptionError.undeliverable("受信端末に鍵を配れませんでした") }
        }
    }

    /// 新しい本文を送る。
    func send(channel: String, id: String, parent: String?, body: String, attachments: [AttachmentInfo] = [], mentions: EncryptedMentions = EncryptedMentions(),
              onRecipients: ((RecipientSummary) -> Void)? = nil) async throws {
        try await serialized {
            try await self.submit(channel: channel, id: id, parent: parent, body: body, revision: 0, edit: nil, attachments: attachments,
                                  mentions: mentions, onRecipients: onRecipients)
        }
    }

    /// 自分の本文を編集する。元の暗号文を復号して、最新の版であることを確かめてから、次の版を送る。
    func edit(_ row: EncryptedRow, body: String, mentions: EncryptedMentions = EncryptedMentions()) async throws {
        guard row.revision < UInt64.max else { throw MessageEncryptionError.editConflict }
        try await serialized {
            try await self.submit(channel: row.channelID, id: row.id, parent: row.parentID, body: body,
                                  revision: row.revision + 1, edit: row, mentions: mentions)
        }
    }

    /// 送信を 1 つずつにする。
    private func serialized(_ operation: @escaping () async throws -> Void) async throws {
        let previous = tail
        let task = Task { @MainActor in
            await previous?.value
            try await operation()
        }
        tail = Task { _ = try? await task.value }
        try await task.value
        // 既存のバックアップ処理が署名・版・失効の検査を行う。
        await uploadBackup()
    }

    private func submit(channel: String, id: String, parent: String?, body: String, revision: UInt64, edit: EncryptedRow?,
                        attachments newAttachments: [AttachmentInfo] = [], mentions: EncryptedMentions = EncryptedMentions(),
                        onRecipients: ((RecipientSummary) -> Void)? = nil) async throws {
        let store = try currentStore()
        var minimum = latestEpochs[channel] ?? 0
        for _ in 0..<3 {
            minimum = max(minimum, latestEpochs[channel] ?? 0)
            let (epoch, devices) = try await recipients(store, channel: channel, minimum: minimum, onRecipients: onRecipients)
            guard epoch >= (latestEpochs[channel] ?? 0) else {
                discard(channel)
                continue
            }
            latestEpochs[channel] = max(latestEpochs[channel] ?? 0, epoch)
            do {
                let nowMs = now()
                // 端末を作り直した（送信者の鍵が変わった）ときも、古いセッションを捨てる。受信側は古い鍵の受信用セッションしか
                // 持たず、`shared` も古い端末の分なので、新しい鍵を配り直せるようにする。
                if let cached = outbound[channel], cached.owner !== store || cached.senderKey != store.curve25519Key
                    || !cached.shared.isSubset(of: Set(devices.map(deviceKey)))
                    || cached.session.needsRotation(nowMs: nowMs, currentEpoch: epoch) {
                    discard(channel)
                }
                let current: SendingSession
                if let existing = outbound[channel] { current = existing } else {
                    current = SendingSession(owner: store, session: try OutboundSession(channelId: channel, epoch: epoch, nowMs: nowMs))
                    outbound[channel] = current
                }
                let pending = devices.filter { !current.shared.contains(deviceKey($0)) }
                try await share(store, outbound: current.session, devices: pending)
                current.shared.formUnion(pending.map(deviceKey))
                guard epoch >= (latestEpochs[channel] ?? 0) else {
                    discard(channel)
                    continue
                }
                // 鍵の配布の応答を待つあいだに、端末が作り直されていたら（送信者の鍵が変わった）、古い鍵で配ったセッションを
                // 新しい鍵で暗号化しない。捨てて、配布先の取得からやり直す。
                if current.senderKey != store.curve25519Key {
                    discard(channel)
                    continue
                }
                var previous: DecryptedMessage?
                if let edit {
                    let decrypted = try store.decrypt(EncryptedMessage(channelId: channel, messageId: id, parentId: parent,
                                                                       userId: store.userID, messageKind: edit.kind, epoch: edit.epoch,
                                                                       revision: revision - 1, ciphertext: edit.ciphertext))
                    guard decrypted.revision == .latest else { throw MessageEncryptionError.editConflict }
                    previous = decrypted
                }
                let kind = previous?.payload.content.kind ?? "message"
                let attachments = previous?.payload.attachments ?? newAttachments
                let encrypted: String
                do { encrypted = try store.encrypt(current.session, senderKey: current.senderKey, message: OutgoingMessage(
                    messageId: id, parentId: parent, revision: revision,
                    content: ZooCrypto.MessageContent(body: body, format: "mfm", kind: kind),
                    mentions: Mentions(userIds: mentions.userIDs, channel: mentions.channel, here: mentions.here), attachments: attachments))
                } catch E2eeStoreError.senderChanged {
                    // 暗号化の直前に端末が作り直された。古い鍵で配ったセッションを捨てて、配布先の取得からやり直す。
                    discard(channel)
                    continue
                }
                let ciphertext = try JSONDecoder().decode(JSONValue.self, from: Data(encrypted.utf8))
                guard case .object = ciphertext else { throw ZooError.invalidResponse }
                var payload: [String: JSONValue] = [
                    "content": .string(""), "kind": .string(kind), "epoch": .number(Double(epoch)),
                    "revision": .number(Double(revision)), "ciphertext": ciphertext
                ]
                if let parent { payload["parent_id"] = .string(parent) }
                if edit == nil {
                    // 添付の行は、サーバーが `attachment_ids` でメッセージに紐付ける（中身の情報は暗号文の中だけ）。
                    if !attachments.isEmpty { payload["attachment_ids"] = .array(attachments.map { .string($0.attachmentId) }) }
                    payload["id"] = .string(id)
                    payload["channel_id"] = .string(channel)
                    // 通知の宛先を作るための平文（サーバーは本文を読めない）。編集では通知を作らないので、付けない。
                    payload["mentions"] = .object(["user_ids": .array(mentions.userIDs.map(JSONValue.string)),
                                                   "channel": .bool(mentions.channel), "here": .bool(mentions.here)])
                }
                do {
                    do {
                        _ = try await request(edit == nil ? "api/messages" : "api/messages/\(id)", method: edit == nil ? "POST" : "PATCH", body: payload)
                    } catch let failure as E2eeAPIError where failure.status == 403 && failure.code == "THREAD_JOIN_REQUIRED" {
                        guard edit == nil, let parent, try await api.rejoinThread(messageID: parent) else { throw failure }
                        // 再参加後も同じ ID と暗号文を一度だけ送る。
                        _ = try await request("api/messages", method: "POST", body: payload)
                    }
                    return
                } catch let failure as E2eeAPIError where failure.code == "E2EE_STALE_EPOCH" {
                    discard(channel)
                    guard let current = failure.currentEpoch else { throw failure }
                    minimum = max(current, epoch)
                    latestEpochs[channel] = max(latestEpochs[channel] ?? 0, minimum)
                }
            } catch {
                // 配布や保存に失敗したセッションを使って次の本文を送らない。
                discard(channel)
                throw error
            }
        }
        discard(channel)
        throw MessageEncryptionError.sendStaleEpoch
    }

    /// 暗号化チャンネルのメッセージを復号する。復号できないときは、理由を `notice` に入れる。
    func decrypt(_ row: EncryptedRow) -> DecryptedBody {
        guard let store = store() else { return DecryptedBody(notice: E2eeText.locked) }
        do {
            let decrypted = try store.decrypt(EncryptedMessage(channelId: row.channelID, messageId: row.id, parentId: row.parentID,
                                                               userId: row.userID, messageKind: row.kind, epoch: row.epoch,
                                                               revision: row.revision, ciphertext: row.ciphertext))
            switch decrypted.revision {
            case .stale: return DecryptedBody(revision: .stale)
            case .conflict: return DecryptedBody(revision: .conflict)
            case .latest:
                let notice: String? = decrypted.origin == .backup ? E2eeText.senderUnauthenticated
                    : (!decrypted.senderVerified ? E2eeText.senderUnverified : nil)
                return DecryptedBody(body: decrypted.payload.content.body, notice: notice, payload: decrypted.payload)
            }
        } catch ZooCryptoError.MissingRoomKey {
            return DecryptedBody(notice: E2eeText.keyMissing)
        } catch {
            return DecryptedBody(notice: E2eeText.invalid)
        }
    }
}
