import Foundation
import Observation
import ZooCrypto

/// 会話の画面が使う、暗号化チャンネルの復号と送信。
@MainActor
protocol ConversationEncryption: AnyObject {
    /// この端末で解錠していて、送受信できる。
    var isReady: Bool { get }
    /// 復号の結果が変わりうる（鍵が届いた、解錠した）たびに増える。
    var revision: Int { get }
    func decrypt(_ message: Message) -> DecryptedBody
    func send(channelID: String, id: String, parentID: String?, body: String) async throws
    func edit(_ message: Message, body: String) async throws
}

/// ログインしているユーザーの E2EE（#685）。端末の管理、初回設定と解錠、暗号化チャンネルの送受信をまとめ、
/// 画面には状態（`status`）と、復号・送信だけを見せる。
@Observable @MainActor
final class E2eeController: ConversationEncryption {
    enum Status: Equatable {
        /// 端末の登録を待っている。
        case starting
        /// SSSS がまだない。初回設定を始める。
        case notSetUp
        /// SSSS はあるが、この端末は解錠していない。
        case locked(LockReason)
        /// 解錠済み。
        case unlocked
    }

    private(set) var status: Status = .starting
    private(set) var revision = 0
    /// 状態を読めなかった（通信の失敗など）。次の登録の成功で消える。
    private(set) var statusError: String?

    @ObservationIgnored let manager: E2eeDeviceManager
    @ObservationIgnored let setup: E2eeSetup
    @ObservationIgnored private let encryption: MessageEncryption
    /// 復号に成功した本文（ID と版ごと）。失敗は鍵が届けば読めるので、覚えない。
    @ObservationIgnored private var decrypted: [String: DecryptedBody] = [:]
    /// パスキーの RP ID（AuthZ のホスト）。
    @ObservationIgnored var rpID = ""
    @ObservationIgnored let passkeys = SystemPasskeyClient()
    /// 直前の再認証の ID トークンを取る（`Reauthenticator`）。
    @ObservationIgnored var reauthenticate: () async throws -> String = { throw ReauthError.browserUnavailable }

    init(manager: E2eeDeviceManager, setup: E2eeSetup, encryption: MessageEncryption) {
        self.manager = manager
        self.setup = setup
        self.encryption = encryption
        manager.onReady = { [weak self] in self?.refreshStatus() }
        manager.onKeysReceived = { [weak self] in self?.revision += 1 }
    }

    /// 本番の組み立て。暗号ストアは Application Support の `e2ee/` に置く。
    static func make(api: APIClient, userID: String, files: CryptoStoreFiles) -> E2eeController {
        let toDevice: (String, @escaping @MainActor ([CryptoStore.ToDeviceRow]) -> Void) async throws -> Void = { deviceID, receive in
            // 暗号文だけだが、同期の記録には残さない（`sync: nil`）。処理済みの印は暗号ストアに置く。
            try await ShapeStream(api: api, sync: nil).observe("api/shapes/e2ee_to_device", query: ["device_id": deviceID],
                                                               as: ToDeviceShapeRow.self, restoreCached: false) { rows in
                receive(ToDeviceShapeRow.ordered(rows ?? []))
            }
        }
        let manager = E2eeDeviceManager(api: api, files: files, userID: userID, observeToDevice: toDevice)
        let setup = E2eeSetup(api: api, userID: userID, store: { try await manager.waitForStore() })
        let encryption = MessageEncryption(store: { manager.readyStore }, api: api)
        return E2eeController(manager: manager, setup: setup, encryption: encryption)
    }

    func start() { manager.start() }
    func stop() async { await manager.stop() }

    var isReady: Bool { status == .unlocked && manager.readyStore != nil }

    /// 状態を読み直す（登録のあと、解錠や初回設定のあと）。解錠済みの端末は、署名し直しなども行う。
    func refreshStatus() {
        Task { @MainActor in
            do {
                let loaded = try await setup.loadStatus()
                status = switch loaded {
                case .notSetUp: .notSetUp
                case .locked(let reason): .locked(reason)
                case .unlocked: .unlocked
                }
                statusError = nil
                revision += 1
            } catch {
                statusError = error.localizedDescription
            }
        }
    }

    // MARK: - 初回設定と解錠（画面から呼ぶ）

    /// 初回設定を始める。返したリカバリーキーを利用者に控えてもらう。
    func beginSetup() throws -> SetupStartHandle { SetupStartHandle(try setup.begin()) }

    /// 初回設定を保存する。再認証が要るときは [E2eeSetupError.reauthRequired] になるので、`reauthenticate` で
    /// ID トークンを取ってからやり直す（同じ鍵のまま）。
    func completeSetup(_ start: SetupStartHandle, passkey: (any PasskeyClient, rpID: String)? = nil, label: String? = nil) async throws {
        if let passkey { try await setup.addSetupPasskey(start.start.session, client: passkey.0, rpID: passkey.rpID, label: label) }
        do { try await setup.completeSetup(start.start.session, reauth: nil) }
        catch E2eeSetupError.reauthRequired {
            let token = try await reauthenticate()
            try await setup.completeSetup(start.start.session, reauth: token)
        }
        finished()
    }

    /// 解錠する。初回設定の再開で再認証が要るときは、ここで取って続きを保存する。
    func unlock(_ method: UnlockMethod) async throws {
        let result = try await setup.unlock(method)
        if result.needsReauth {
            let token = try await reauthenticate()
            try await setup.saveRemaining(result.session, secretsRevision: result.secretsRevision, reauth: token)
        }
        finished()
    }

    private func finished() {
        status = .unlocked
        statusError = nil
        decrypted = [:]
        revision += 1
        refreshStatus()
    }

    // MARK: - 復号と送信

    private func row(_ message: Message) -> EncryptedRow? {
        guard let ciphertext = message.ciphertext, let epoch = message.epoch, let revision = message.revision else { return nil }
        return EncryptedRow(channelID: message.channel_id, id: message.id, parentID: message.parent_id, userID: message.user_id,
                            kind: message.kind ?? "message", epoch: epoch.value, revision: revision.value, ciphertext: ciphertext.value)
    }

    /// 暗号化チャンネルのメッセージを復号する。暗号文がない（平文の）行は、表示しない。
    func decrypt(_ message: Message) -> DecryptedBody {
        guard let row = row(message) else { return DecryptedBody(notice: E2eeText.unencrypted) }
        let key = "\(row.id)|\(row.revision)|\(row.ciphertext.hashValue)"
        if let cached = decrypted[key] { return cached }
        let result = encryption.decrypt(row)
        if result.body != nil { decrypted[key] = result }
        return result
    }

    func send(channelID: String, id: String, parentID: String?, body: String) async throws {
        try await encryption.send(channel: channelID, id: id, parent: parentID, body: body)
    }

    func edit(_ message: Message, body: String) async throws {
        guard let row = row(message) else { throw MessageEncryptionError.editForbidden }
        try await encryption.edit(row, body: body)
    }

}

/// 初回設定の途中の状態（SwiftUI の `@State` に持てるように包む）。
final class SetupStartHandle: @unchecked Sendable {
    let start: SetupStart
    var recoveryKey: String { start.recoveryKey }
    init(_ start: SetupStart) { self.start = start }
}
