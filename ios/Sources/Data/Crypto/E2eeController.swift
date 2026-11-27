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
    func send(channelID: String, id: String, parentID: String?, body: String, attachments: [AttachmentInfo], mentions: EncryptedMentions,
              onRecipients: @escaping (MessageEncryption.RecipientSummary) -> Void) async throws
    func edit(_ message: Message, body: String, mentions: EncryptedMentions) async throws
    /// 送る前の、配布先の要約（前回から増えた配布先と、鍵を配れない相手）。確かめられなければ `nil`。
    func recipientSummary(channelID: String) async -> MessageEncryption.RecipientSummary?
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
    /// 解錠・初回設定が終わるたびに進める（古い読み直しの失敗が、新しい解錠を取り消さないため）。
    @ObservationIgnored private var unlockGeneration = 0
    /// SSSS が書き換えられたと分かったあと、解錠し直すまで立てる。保留中の古い読み直しが、解錠済みを書き戻さないため。
    @ObservationIgnored private var secretsChanged = false
    /// 鍵スロット（パスキーとリカバリーキー）の追加と削除。本番は `make` で作る。
    @ObservationIgnored var keySlots: E2eeKeySlots?
    /// 失効（端末の削除のあとの鍵の作り直し）。本番は `make` で作る。
    @ObservationIgnored var revocation: E2eeRevocation?
    /// ログアウトで暗号ストアを消すための置き場と、接続先（削除待ちの照合に使う）。本番は `make` で作る。
    @ObservationIgnored var files: CryptoStoreFiles?
    @ObservationIgnored var serverKey = ""
    @ObservationIgnored var userIDForStore = ""
    /// 他のユーザーの検証（SAS）。本番は `make` で作る。
    @ObservationIgnored var verification: E2eeVerification?
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
        manager.onReady = { [weak self] in
            self?.refreshEncryptionHistory(retry: true)
            self?.rememberEncryptedChannels([])
            self?.onEncryptionHistoryReloaded?()
            self?.refreshStatus()
        }
        manager.onKeysReceived = { [weak self] in
            guard let self else { return }
            // 新しい鍵（検証済みの鍵が、検証できなかった鍵を置き換えることがある）で、同じ行の復号の結果が変わりうるので、
            // 復号のキャッシュを捨てて、読み直させる。
            self.decrypted = [:]
            self.revision += 1
            // 届いた部屋の鍵（受信用のセッション）を鍵バックアップへ送る。失敗しても、次の起動時や次の鍵で再び試す。
            let startedAt = self.unlockGeneration
            Task { @MainActor in
                // SSSS が書き換えられていたら（別の端末で失効・解錠し直した）、解錠済みの表示を残さず、状態を読み直す。
                if case .conflict(let code)? = try? await self.setup.uploadKeyBackup(), code == "E2EE_SSSS_CONFLICT",
                   startedAt == self.unlockGeneration {  // 送っているあいだに解錠し直していたら、その解錠を取り消さない。
                    // 競合を検出した時点で、解錠済みの表示を解く（読み直しに失敗しても残さない）。読み直せれば、正しい状態に戻る。
                    self.secretsChanged = true
                    self.status = .locked(.mismatch)
                    self.refreshStatus()
                }
            }
        }
    }

    /// 本番の組み立て。暗号ストアは Application Support の `e2ee/` に置く。
    static func make(api: APIClient, userID: String, files: CryptoStoreFiles) -> E2eeController {
        let toDevice: (String, @escaping @MainActor ([CryptoStore.ToDeviceRow]) -> Void) async throws -> Void = { deviceID, receive in
            // 暗号文だけだが、同期の記録には残さない（`sync: nil`）。処理済みの印は暗号ストアに置く。
            try await ShapeStream(api: api, sync: nil).observe("api/shapes/e2ee_to_device", query: ["device_id": deviceID],
                                                               as: ToDeviceShapeRow.self, restoreCached: false) { rows in
                // 再取得開始の nil は空の完全な Shape ではないため、処理済みの印を回収しない。
                if let rows { receive(ToDeviceShapeRow.ordered(rows)) }
            }
        }
        let manager = E2eeDeviceManager(api: api, files: files, userID: userID, observeToDevice: toDevice)
        let setup = E2eeSetup(api: api, userID: userID, store: { try await manager.waitForStore() })
        let encryption = MessageEncryption(store: { manager.readyStore }, api: api, uploadBackup: {
            setup.scheduleKeyBackup()
        })
        let controller = E2eeController(manager: manager, setup: setup, encryption: encryption)
        controller.devicesAPI = api
        controller.files = files
        controller.serverKey = api.settings.server
        controller.userIDForStore = userID
        let verification = E2eeVerification(store: { manager.readyStore }, api: api)
        controller.verification = verification
        controller.keySlots = E2eeKeySlots(api: api, userID: userID)
        controller.revocation = E2eeRevocation(store: { manager.readyStore }, api: api, setup: setup, userID: userID)
        manager.onVerifications = { verification.handle($0) }
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

    /// 端末を作り直したので、終わっていなかった失効の記録を捨てた（失効を、最初からやり直してもらう）。
    var revocationWasDropped: Bool { manager.readyStore?.revocationDropped == true }
    func acknowledgeRevocationDropped() { try? manager.readyStore?.acknowledgeRevocationDropped() }

    @ObservationIgnored private var observedEncryptedChannels = Set<String>()
    @ObservationIgnored private var readEncryptionHistory = false
    @ObservationIgnored private var encryptionHistoryAvailable = false
    @ObservationIgnored private var canPersistEncryptionHistory = false
    @ObservationIgnored private var durableEncryptedChannels = Set<String>()
    @ObservationIgnored var onEncryptionHistoryReloaded: (() -> Void)?

    private func refreshEncryptionHistory(retry: Bool = false) {
        if (!readEncryptionHistory || (retry && !canPersistEncryptionHistory)), let files {
            readEncryptionHistory = true
            do {
                durableEncryptedChannels = try files.loadEncryptionHistory(userID: userIDForStore)
                observedEncryptedChannels.formUnion(durableEncryptedChannels)
                canPersistEncryptionHistory = true
                encryptionHistoryAvailable = true
            } catch {
                encryptionHistoryAvailable = false
            }
        }
        if let store = manager.readyStore { observedEncryptedChannels.formUnion(store.encryptedChannels) }
    }

    /// 鍵や端末登録の準備を待たず、観測した暗号化チャンネルを永続化する。
    func rememberEncryptedChannels(_ channels: [Channel]) {
        refreshEncryptionHistory()
        observedEncryptedChannels.formUnion(channels.filter { $0.encrypted == true && !$0.encryptionIsLocalOverride }.map(\.id))
        if let files, canPersistEncryptionHistory,
           !encryptionHistoryAvailable || durableEncryptedChannels != observedEncryptedChannels {
            do {
                try files.saveEncryptionHistory(observedEncryptedChannels, userID: userIDForStore)
                durableEncryptedChannels = observedEncryptedChannels
                encryptionHistoryAvailable = true
            } catch { encryptionHistoryAvailable = false }
        }
        if let store = manager.readyStore, !observedEncryptedChannels.isEmpty {
            try? store.rememberEncryptedChannels(Array(observedEncryptedChannels))
        }
    }

    /// ローカルの履歴を確かめられない間も、平文への切替を認めない。
    func isDowngraded(_ channel: Channel) -> Bool {
        refreshEncryptionHistory()
        guard channel.encrypted != true || channel.encryptionIsLocalOverride else { return false }
        return observedEncryptedChannels.contains(channel.id) || (files != nil && !encryptionHistoryAvailable)
    }

    /// 待っているパスキーの要求を取り消す。
    func cancelPasskey() { (passkeys as? SystemPasskeyClient)?.cancel() }

    func start() {
        manager.start()
        Task { @MainActor [weak self] in await self?.retryPendingDeviceDeletions() }
        // 10 分を過ぎた検証の取りやめと、残したメッセージの試し直し。
        expireTask?.cancel()
        expireTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(5))
                await self?.verification?.expire()
            }
        }
    }
    @ObservationIgnored private var expireTask: Task<Void, Never>?

    func stop() async {
        setup.cancelScheduledBackup()
        expireTask?.cancel()
        expireTask = nil
        await manager.stop()
        EncryptedAttachmentFiles.shared.removeAll()
    }

    var isReady: Bool { status == .unlocked && manager.readyStore != nil }

    /// 状態を読み直す（登録のあと、解錠や初回設定のあと）。解錠済みの端末は、署名し直しなども行う。
    func refreshStatus() {
        refreshGeneration += 1
        let generation = refreshGeneration
        let unlockGeneration = self.unlockGeneration
        Task { @MainActor in
            do {
                // 解錠済みなら、鍵バックアップの送信と取り込みも行う（失敗しても状態の表示は止めない）。
                let checked = try await setup.runStartupCheck()
                let loaded = if let checked { checked } else { try await setup.loadStatus() }
                // 解錠済みでないと分かったら（競合のあとの読み直しなど）、解錠し直すまで、ほかの読み直しの「解錠済み」を適用しない。
                // その結果自身が古くて捨てられる場合も、これは覚えておく（解錠し直していれば、覚えない）。
                if loaded != .unlocked, unlockGeneration == self.unlockGeneration {
                    secretsChanged = true
                    // 古い読み直しでも、解錠済みでない状態は表示に反映する（元の「解錠済み」を残さない）。
                    status = switch loaded {
                    case .notSetUp: .notSetUp
                    case .locked(let reason): .locked(reason)
                    case .unlocked: status
                    }
                }
                // あとから始めた読み直しがあれば、古い結果で状態を上書きしない。
                guard generation == refreshGeneration else { return }
                // 競合でロックしたあとは、解錠し直すまで、古い読み直しの「解錠済み」を適用しない。
                if secretsChanged, loaded == .unlocked { return }
                status = switch loaded {
                case .notSetUp: .notSetUp
                case .locked(let reason): .locked(reason)
                case .unlocked: .unlocked
                }
                statusError = nil
                revision += 1
            } catch {
                // 古い読み直しの通信エラーは表示しない（あとから始めた読み直しが成功して消したエラーを、再表示しない）。
                // SSSS の競合は、下で別に扱う。
                if generation == refreshGeneration { statusError = error.localizedDescription }
                // SSSS が書き換えられたと分かったのに読み直せないときは、解錠済みの表示を残さない（解錠し直しが要る）。
                // あとから始めた読み直しがあっても、競合を検出した結果は捨てない（そちらも失敗すれば、ロックが残る）。
                // 競合のあとに、利用者が解錠し直していたら、その結果は取り消さない。
                if case E2eeSetupError.secretsChanged = error, unlockGeneration == self.unlockGeneration {
                    secretsChanged = true
                    status = .locked(.mismatch)
                    statusError = "暗号化の鍵が別の端末で更新されました。解錠し直してください。"
                }
            }
        }
    }


    // MARK: - ログアウト（docs/e2ee.md の「ログアウト」）

    /// ログアウトの前の確認。動いているうちに、送れていない鍵バックアップを 5 秒まで送ろうとし、処理を止めて、暗号ストアを
    /// 開き直して、まだバックアップに送れていない受信用セッションの数を返す（数えられなかったときは `nil`）。
    /// 取りやめるときは `cancelLogout()` で処理に戻す。
    func prepareLogout() async -> Int? {
        _ = await e2eeWithTimeout(.seconds(5)) { [setup] in (try? await setup.uploadKeyBackup()) != nil ? true : nil }
        await stop()
        guard let files, let opened = try? CryptoStore.open(files, userID: userIDForStore, create: false) else { return nil }
        let store = opened.store
        defer { store.close() }
        return store.pendingBackupCount()
    }

    /// ログアウトを取りやめた。処理に戻る。
    func cancelLogout() { start() }

    /// ログアウトする：端末をサーバーから消し（5 秒まで）、暗号ストアを消す。消せなかった端末は削除待ちに残し、同じ接続先と
    /// ユーザーの次のログインで消し直す。端末の削除にはトークンが要るので、認証情報を消す前に呼ぶ。
    func finishLogout() async throws {
        await stop()
        defer { EncryptedAttachmentFiles.shared.removeAll() }
        guard let files else { return }
        let metadata = try files.metadata(userID: userIDForStore)
        if let metadata {
            // 登録の応答だけを失った端末も、サーバーに残っている可能性がある。
            let devices = Set((metadata.pending_deletions ?? []) + [metadata.device_id]).sorted()
            let pending = PendingDeviceDeletions(root: files.directory.deletingLastPathComponent())
            let entries = devices.map { PendingDeviceDeletion(server: serverKey, user: userIDForStore, device: $0) }
            var unrecorded = Set(entries.filter { !pending.add($0) }.map(\.device))
            _ = await e2eeWithTimeout(.seconds(5)) { [devicesAPI] () -> Bool? in
                guard let api = devicesAPI else { return false }
                for entry in entries {
                    if Task.isCancelled { return false }
                    do { try await api.removeDevice(entry.device) }
                    catch { continue }
                    unrecorded.remove(entry.device)
                    pending.remove(entry)
                }
                return true
            }
            if !unrecorded.isEmpty {
                // 再試行先を失わないよう、ローカル状態を残して失敗を画面へ返す。
                throw CocoaError(.fileWriteUnknown)
            }
        }
        try files.removeLocalState()
    }

    /// 前のログアウトで消せなかった、この接続先とユーザーの端末を消し直す（ログインのあと、裏で行う）。
    /// 削除済みIDには成功が返る。404は遅延した登録がまだ届いていない可能性があるので残す。
    func retryPendingDeviceDeletions() async {
        guard let files, let api = devicesAPI else { return }
        let pending = PendingDeviceDeletions(root: files.directory.deletingLastPathComponent())
        for entry in pending.load() ?? [] where entry.server == serverKey && entry.user == userIDForStore {
            let done = await e2eeWithTimeout(.seconds(5)) { () -> Bool? in
                do { try await api.removeDevice(entry.device); return true }
                catch { return false }
            }
            if done == true { pending.remove(entry) }
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

    /// E2EE のリセットを始める（解錠できないとき。他人が作った SSSS を含む）。新しい鍵とリカバリーキーを作る。
    func beginReset() async throws -> ResetHandle {
        let started = try await setup.beginReset()
        return ResetHandle(start: SetupStartHandle(started.start), expectation: started.expectation)
    }

    /// リセットに、パスキー（PRF）のスロットを足す。
    func addResetPasskey(_ reset: ResetHandle, label: String?) async throws {
        try await setup.addSetupPasskey(reset.start.start.session, client: passkeys, rpID: rpID, label: label)
    }

    /// リセットを保存する。直前の再認証が要るので、ここで取ってから保存する（足りなければ取り直す）。
    func completeReset(_ reset: ResetHandle) async throws {
        var attempts = 0
        while true {
            let token = try await reauthenticate()
            do {
                try await setup.completeReset(reset.start.start.session, expectation: reset.expectation, reauth: token)
                break
            } catch E2eeSetupError.reauthRequired where attempts < 1 { attempts += 1 }
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
        unlockGeneration += 1
        secretsChanged = false
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

    func send(channelID: String, id: String, parentID: String?, body: String, attachments: [AttachmentInfo], mentions: EncryptedMentions,
              onRecipients: @escaping (MessageEncryption.RecipientSummary) -> Void) async throws {
        try await encryption.send(channel: channelID, id: id, parent: parentID, body: body, attachments: attachments,
                                  mentions: mentions, onRecipients: onRecipients)
    }

    func recipientSummary(channelID: String) async -> MessageEncryption.RecipientSummary? {
        guard isReady else { return nil }
        return try? await encryption.summary(channel: channelID)
    }

    func edit(_ message: Message, body: String, mentions: EncryptedMentions) async throws {
        guard let row = row(message) else { throw MessageEncryptionError.editForbidden }
        try await encryption.edit(row, body: body, mentions: mentions)
    }

}

/// 初回設定の途中の状態（SwiftUI の `@State` に持てるように包む）。
/// リセットの途中の状態（新しい鍵と、前提にしたサーバーの SSSS）。
final class ResetHandle: @unchecked Sendable {
    let start: SetupStartHandle
    let expectation: E2eeSetup.ResetExpectation
    init(start: SetupStartHandle, expectation: E2eeSetup.ResetExpectation) { self.start = start; self.expectation = expectation }
    var recoveryKey: String { start.recoveryKey }
}

final class SetupStartHandle: @unchecked Sendable {
    let start: SetupStart
    var recoveryKey: String { start.recoveryKey }
    init(_ start: SetupStart) { self.start = start }
}
