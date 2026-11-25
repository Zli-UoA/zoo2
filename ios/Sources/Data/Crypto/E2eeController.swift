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
    func send(channelID: String, id: String, parentID: String?, body: String, attachments: [AttachmentInfo]) async throws
    func edit(_ message: Message, body: String) async throws
}

/// ログインしているユーザーの E2EE（#685）。端末の管理、初回設定と解錠、暗号化チャンネルの送受信をまとめ、
/// 画面には状態（`status`）と、復号・送信だけを見せる。
/// 復号の結果を使い回す条件。検証に影響する列をすべて含める（暗号文は値そのもので比べる）。
private struct DecryptedKey: Hashable {
    let channelID: String, id: String, parentID: String?, userID: String, kind: String
    let epoch: UInt64, revision: UInt64, ciphertext: String
    init(row: EncryptedRow) {
        channelID = row.channelID; id = row.id; parentID = row.parentID; userID = row.userID; kind = row.kind
        epoch = row.epoch; revision = row.revision; ciphertext = row.ciphertext
    }
}

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
    @ObservationIgnored private var decrypted: [DecryptedKey: DecryptedBody] = [:]
    /// メッセージごとの、確認できた最新の版の本文。古い版や、同じ版で中身の違う行が届いたときに、この本文を警告つきで出す。
    @ObservationIgnored private var confirmed: [String: (revision: UInt64, result: DecryptedBody)] = [:]
    /// パスキーの RP ID（AuthZ のホスト）。
    @ObservationIgnored private var refreshGeneration = 0
    /// 端末一覧の API（本番は `APIClient`）。
    @ObservationIgnored var devicesAPI: (any E2eeOwnDevicesAPI)?
    @ObservationIgnored var rpID = ""
    @ObservationIgnored var passkeys: any PasskeyClient = SystemPasskeyClient()
    /// 直前の再認証の ID トークンを取る（`Reauthenticator`）。
    @ObservationIgnored var reauthenticate: () async throws -> String = { throw ReauthError.browserUnavailable }

    init(manager: E2eeDeviceManager, setup: E2eeSetup, encryption: MessageEncryption) {
        self.manager = manager
        self.setup = setup
        self.encryption = encryption
        manager.onReady = { [weak self] in self?.refreshStatus() }
        manager.onKeysReceived = { [weak self] in
            guard let self else { return }
            self.revision += 1
            // 届いた部屋の鍵（受信用のセッション）を鍵バックアップへ送る。失敗しても、次の起動時や次の鍵で再び試す。
            Task { @MainActor in
                // SSSS が書き換えられていたら（別の端末で失効・解錠し直した）、解錠済みの表示を残さず、状態を読み直す。
                if case .conflict(let code)? = try? await self.setup.uploadKeyBackup(), code == "E2EE_SSSS_CONFLICT" { self.refreshStatus() }
            }
        }
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
        let controller = E2eeController(manager: manager, setup: setup, encryption: encryption)
        controller.devicesAPI = api
        return controller
    }

    /// プレビュー用：準備の失敗を、画面とテストから見えるようにする。
    func setPreviewFailure(_ message: String) { statusError = message }

    /// この端末の `device_id`（登録後。端末一覧で「この端末」を示す）。
    var currentDeviceID: String? { manager.readyStore?.deviceID }

    func ownDevices() async throws -> [OwnDevice] { try await devicesAPI?.ownDevices() ?? [] }
    func renameDevice(_ id: String, to name: String) async throws { try await devicesAPI?.renameDevice(id, to: name) }
    /// 別の端末を削除する（この端末は消さない）。
    func removeDevice(_ id: String) async throws {
        guard id.lowercased() != currentDeviceID?.lowercased() else { return }
        try await devicesAPI?.removeDevice(id)
    }

    /// 待っているパスキーの要求を取り消す。
    func cancelPasskey() { (passkeys as? SystemPasskeyClient)?.cancel() }

    func start() { manager.start() }
    func stop() async {
        await manager.stop()
        EncryptedAttachmentFiles.shared.removeAll()
    }

    var isReady: Bool { status == .unlocked && manager.readyStore != nil }

    /// 状態を読み直す（登録のあと、解錠や初回設定のあと）。解錠済みの端末は、署名し直しなども行う。
    func refreshStatus() {
        refreshGeneration += 1
        let generation = refreshGeneration
        Task { @MainActor in
            do {
                // 解錠済みなら、鍵バックアップの送信と取り込みも行う（失敗しても状態の表示は止めない）。
                let checked = try await setup.runStartupCheck()
                let loaded = if let checked { checked } else { try await setup.loadStatus() }
                // あとから始めた読み直しがあれば、古い結果で状態を上書きしない。
                guard generation == refreshGeneration else { return }
                status = switch loaded {
                case .notSetUp: .notSetUp
                case .locked(let reason): .locked(reason)
                case .unlocked: .unlocked
                }
                statusError = nil
                revision += 1
            } catch {
                statusError = error.localizedDescription
                // SSSS が書き換えられたと分かったのに読み直せないときは、解錠済みの表示を残さない（解錠し直しが要る）。
                if case E2eeSetupError.secretsChanged = error, generation == refreshGeneration {
                    status = .locked(.mismatch)
                    statusError = "暗号化の鍵が別の端末で更新されました。解錠し直してください。"
                }
            }
        }
    }

    // MARK: - 初回設定と解錠（画面から呼ぶ）

    /// 初回設定を始める。返したリカバリーキーを利用者に控えてもらう。
    func beginSetup() throws -> SetupStartHandle { SetupStartHandle(try setup.begin()) }

    /// 初回設定に、パスキー（PRF）のスロットを足す。利用者がパスキーを作る（選ぶ）。保存の前なので、サーバーには何も書かない。
    func addSetupPasskey(_ start: SetupStartHandle, label: String?) async throws {
        try await setup.addSetupPasskey(start.start.session, client: passkeys, rpID: rpID, label: label)
    }

    /// 初回設定を保存する。再認証が要るときは、ここで ID トークンを取ってから、同じ鍵のまま保存し直す。
    func completeSetup(_ start: SetupStartHandle) async throws {
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
        // 検証に使う列（送信者、チャンネル、返信先、種別、epoch、暗号文）がすべて同じときだけ、復号の結果を使い回す。
        let key = DecryptedKey(row: row)
        if let cached = decrypted[key] {
            // 確認できた最新の版より古い版が再送されたら、キャッシュを返さず、最新の本文を警告つきで出す。
            if let known = confirmed[row.id], known.revision > row.revision {
                var shown = known.result
                shown.notice = E2eeText.revisionStale
                return shown
            }
            return cached
        }
        let result = encryption.decrypt(row)
        if result.body != nil {
            decrypted[key] = result
            if (confirmed[row.id]?.revision ?? 0) <= row.revision || confirmed[row.id] == nil { confirmed[row.id] = (row.revision, result) }
            return result
        }
        // 古い版（`stale`）や、同じ版で中身の違う行（`conflict`）は、この行の本文ではなく、確認できた最新の本文を、
        // 警告つきで出す（Web と同じ）。まだ確認できた本文がなければ、警告だけ。
        if result.revision != .latest, let known = confirmed[row.id] {
            var shown = known.result
            shown.notice = result.revision == .stale ? E2eeText.revisionStale : E2eeText.revisionConflict
            return shown
        }
        return result
    }

    func send(channelID: String, id: String, parentID: String?, body: String, attachments: [AttachmentInfo]) async throws {
        try await encryption.send(channel: channelID, id: id, parent: parentID, body: body, attachments: attachments)
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
