import Foundation
import Observation

@Observable @MainActor
final class AppSession {
    static let shared = configured()
    private(set) var userID: String?
    private(set) var workspace: WorkspaceStore?
    var connecting: Bool { connection != nil }
    private(set) var waitingForCacheCleanup = false
    var restoring: Bool { connection?.interactive == false }
    private(set) var isStarting = true
    private(set) var deviceAuthorization: DeviceAuthorizationPrompt?
    var error: String?
    let notifications: NotificationController
    @ObservationIgnored weak var systemCalls: SystemCallController?
    private let authentication: Authentication
    private let api: APIClient
    private var connection: (task: Task<Void, Never>, interactive: Bool)?
    private var generation = UUID()
    private var syncCache: SyncCache?
    private let openSyncCache: @Sendable () async throws -> SyncCache
    private var syncSession: SyncSession?
    private var cacheCleanup: Task<Void, Error>?
    private var needsCacheCleanup = false
    private var destinationDelivery: (id: UUID, target: PushDestination, task: Task<Bool, Never>)?
    private var validatedSession = false

    init(settings: ConnectionSettings = ConnectionSettings(), transport: HTTPTransport = HTTPTransport(),
         notificationStore: any PushReceiptStore = KeychainPushReceiptStore(),
         previewKeys: any NotificationPreviewKeyStorage = NotificationPreviewKeyStore(), syncCache: SyncCache? = nil,
         openSyncCache: @escaping @Sendable () async throws -> SyncCache = { try await SyncCache.open(inMemory: AppSession.isUITesting) }) {
        self.syncCache = syncCache
        self.openSyncCache = openSyncCache
        authentication = Authentication(settings: settings, transport: transport)
        api = APIClient(settings: settings, auth: authentication, transport: transport)
        notifications = NotificationController(transport: transport, receiptStore: notificationStore, previewKeys: previewKeys)
    }

    static func configured() -> AppSession {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--uitesting-workspace") {
            URLProtocol.registerClass(PreviewEmojiURLProtocol.self)
            let session = AppSession(settings: ConnectionSettings(server: "https://ui.test.invalid", clientID: "ui-test"),
                                     notificationStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys())
            let client = PreviewChatClient()
            session.isStarting = false
            session.userID = client.currentUser.id
            session.validatedSession = true
            let calls = CallStore(client: client, userID: client.currentUser.id, makeMedia: { PreviewCallMedia() }, permission: { true })
            session.workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil, calls: calls)
            if let workspace = session.workspace, ProcessInfo.processInfo.arguments.contains("--uitesting-e2ee") {
                workspace.e2ee = E2eeController.makePreview(
                    userID: client.currentUser.id,
                    alreadySetUp: ProcessInfo.processInfo.arguments.contains("--uitesting-e2ee-locked"))
            }
            if let workspace = session.workspace, ProcessInfo.processInfo.arguments.contains("--uitesting-attachments") {
                Task { await client.prepareAttachmentDraft(in: workspace) }
            }
            if let workspace = session.workspace {
                client.runShortcutPreview(in: workspace)
            }
            return session
        }
        if isUITesting {
            let session = AppSession(settings: ConnectionSettings(server: "https://ui.test.invalid", clientID: "ui-test"),
                              notificationStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys())
            session.isStarting = false
            return session
        }
        #endif
        NativeCallLiveActivity.endInterrupted()
        return AppSession()
    }

    static var isUITesting: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("--uitesting") || ProcessInfo.processInfo.arguments.contains("--uitesting-workspace") ||
            ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
        #else
        false
        #endif
    }

    @discardableResult
    func startLogin() -> Task<Void, Never>? {
        guard !connecting, userID == nil else { return nil }
        notifications.discardAccountNotifications()
        return beginConnection(interactive: true)
    }

    @discardableResult
    private func beginConnection(interactive: Bool) -> Task<Void, Never> {
        generation = UUID()
        let epoch = generation
        let task = Task {
            guard generation == epoch, !Task.isCancelled else { return }
            await connect(interactive: interactive)
        }
        connection = (task, interactive)
        return task
    }

    func cancelLogin() {
        guard let connection else { return }
        if connection.interactive { logout() } else { cancelConnection() }
    }

    private func connect(interactive: Bool) async {
        let epoch = generation
        error = nil
        defer {
            if generation == epoch {
                deviceAuthorization = nil
                connection = nil
            }
        }
        do {
            do {
                if let cacheCleanup {
                    waitingForCacheCleanup = true
                    defer { if generation == epoch { waitingForCacheCleanup = false } }
                    try await cacheCleanup.value
                }
            }
            catch {
                if generation == epoch { cacheCleanup = nil }
                throw error
            }
            try Task.checkCancellation()
            guard generation == epoch else { return }
            cacheCleanup = nil
            if syncCache == nil {
                let opened = try await openSyncCache()
                try Task.checkCancellation()
                guard generation == epoch else { return }
                syncCache = opened
            }
            guard let syncCache else { throw ZooError.invalidResponse }
            if needsCacheCleanup {
                try await syncCache.forget(connection: SyncCache.connectionKey(api.settings))
                try Task.checkCancellation()
                guard generation == epoch else { return }
                needsCacheCleanup = false
            }
            if interactive {
                try await syncCache.forget(connection: SyncCache.connectionKey(api.settings))
                try await authentication.login { prompt in deviceAuthorization = prompt }
                deviceAuthorization = nil
            } else if workspace == nil, try authentication.hasSession,
                      let saved = try await syncCache.profile(connection: SyncCache.connectionKey(api.settings)) {
                try await installWorkspace(saved, cache: syncCache, epoch: epoch)
            }
            try Task.checkCancellation()
            let user: Person = try await api.get("api/users/me")
            try Task.checkCancellation()
            guard generation == epoch else { return }
            if userID != user.id || workspace == nil {
                try await installWorkspace(user, cache: syncCache, epoch: epoch)
            }
            validatedSession = true
            if !Self.isUITesting { ZooShortcuts.updateAppShortcutParameters() }
            await notifications.resume()
            await deliverDestination()
        } catch {
            guard generation == epoch, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error {
                logout()
                if self.error == nil { self.error = error.localizedDescription }
                return
            } else if isOfflineFailure(error), workspace != nil {
                workspace?.error = "オフラインです。保存済みのデータを表示しています。"
                return
            }
            self.error = error.localizedDescription
        }
    }

    private func installWorkspace(_ user: Person, cache: SyncCache, epoch: UUID) async throws {
        try Task.checkCancellation()
        guard generation == epoch else { throw CancellationError() }
        let lease = try await cache.activate(connection: SyncCache.connectionKey(api.settings), user: user)
        guard generation == epoch, !Task.isCancelled else {
            try await cache.revoke(lease)
            throw CancellationError()
        }
        if workspace != nil { systemCalls?.disconnect() }
        workspace?.shutdown()
        let sync = SyncSession(cache: cache, lease: lease)
        syncSession = sync
        userID = user.id
        let client = NativeChatClient(api: api, sync: sync)
        let journal = KeychainCallSessionJournal(account: SyncCache.connectionKey(api.settings) + ":" + user.id)
        let calls = CallStore(client: client, userID: user.id, sessionJournal: Self.isUITesting ? nil : journal)
        let workspace = WorkspaceStore(client: client, user: user, calls: calls)
        if !Self.isUITesting {
            do {
                let files = try CryptoStoreFiles.ios(root: CryptoStoreFiles.defaultRoot(), server: api.settings.server, userID: user.id)
                let controller = E2eeController.make(api: api, userID: user.id, files: files)
                let reauthenticator = Reauthenticator(auth: authentication)
                controller.reauthenticate = { try await reauthenticator.idToken() }
                controller.verification?.reauthenticate = { try await reauthenticator.idToken() }
                controller.rpID = URL(string: api.settings.issuer)?.host ?? ""
                workspace.e2ee = controller
                controller.start()
            } catch {
                // 以前の暗号化履歴も確認できないため、平文送信へ切り替えない。
                workspace.encryptionStorageUnavailable = true
                workspace.error = "暗号化の保存領域を初期化できませんでした。安全を確認できるまで送信できません。アプリを開き直すと再び試します。"
            }
        }
        sync.onChannelRevoked = { [weak workspace] id, channel in workspace?.revokeChannel(id, retaining: channel) }
        sync.onChannelAuthorized = { [weak workspace] id in workspace?.calls.authorizeChannel(id) }
        workspace.onExpiredSession = { [weak self, weak workspace] in
            guard let self, let workspace, self.workspace === workspace else { return }
            logout()
            if error == nil { error = ZooError.expiredSession.localizedDescription }
        }
        workspace.onPeopleChanged = { [weak workspace] in
            if !Self.isUITesting {
                ZooShortcuts.updateAppShortcutParameters()
                SiriVocabulary.update(workspace)
            }
        }
        self.workspace = workspace
        systemCalls?.connect(workspace)
        notifications.connect(api: api, userID: user.id)
    }

    private func cancelConnection() {
        generation = UUID()
        connection?.task.cancel()
        connection = nil
        waitingForCacheCleanup = false
        deviceAuthorization = nil
        error = nil
    }

    /// 暗号鍵をまだバックアップに送れていないまま、ログアウトしようとしている（確認の対象。`nil` なら確認なし）。
    struct LogoutPrompt: Equatable { let unbackedSessions: Int? }
    var logoutPrompt: LogoutPrompt?
    /// ログアウトの確認を始めたときのセッション（確認のあいだに別のセッションになったら、何もしない）。
    @ObservationIgnored private var loggingOutWorkspace: WorkspaceStore?

    /// 利用者のログアウト。暗号化があれば、送れていない鍵を確かめる（0 でなければ確認を出す）。
    func beginLogout() async {
        guard let workspace, let e2ee = workspace.e2ee else { logout(); return }
        // 確認を出しているあいだにセッションが終わったり、別のセッションになったりしたら、古い確認では何もしない。
        error = nil
        loggingOutWorkspace = workspace
        let count = await e2ee.prepareLogout()
        guard self.workspace === workspace else { return }
        if count == 0 { await completeLogout() } else { logoutPrompt = LogoutPrompt(unbackedSessions: count) }
    }

    func confirmLogout() async { await completeLogout() }

    func cancelLogout() {
        logoutPrompt = nil
        loggingOutWorkspace?.e2ee?.cancelLogout()
        loggingOutWorkspace = nil
    }

    private func completeLogout() async {
        // 確認した時のセッションと、今のセッションが同じときだけ行う（違えば、確認は古いので、何もしない）。
        // 確認の持ち主が違うときは、新しいセッションの確認状態を消さない。
        guard let target = loggingOutWorkspace, target === workspace else { return }
        // 端末の削除にはトークンが要るので、認証情報を消す前に、端末を消して暗号ストアを消す。
        do { try await target.e2ee?.finishLogout() }
        catch {
            guard target === workspace, loggingOutWorkspace === target else { return }
            logoutPrompt = nil
            self.error = "暗号化の保存情報を削除できませんでした。もう一度ログアウトしてください。"
            return
        }
        guard target === workspace, loggingOutWorkspace === target else { return }
        logoutPrompt = nil
        loggingOutWorkspace = nil
        logout()
    }

    func logout() {
        logoutPrompt = nil
        loggingOutWorkspace = nil
        cancelConnection()
        destinationDelivery?.task.cancel()
        destinationDelivery = nil
        systemCalls?.disconnect()
        workspace?.shutdown()
        workspace = nil
        userID = nil
        validatedSession = false
        if !Self.isUITesting {
            ZooShortcuts.updateAppShortcutParameters()
            SiriVocabulary.update(nil)
        }
        notifications.logout()
        do { try authentication.logout() }
        catch { self.error = error.localizedDescription }
        if let syncSession {
            needsCacheCleanup = true
            cacheCleanup = syncSession.close()
        }
        self.syncSession = nil
    }

    func resume() async {
        defer {
            isStarting = false
            if !Self.isUITesting { SiriVocabulary.refreshAuthorization(workspace) }
        }
        if userID == nil || !validatedSession {
            guard !connecting else { return }
            var epoch = generation
            do {
                if try authentication.hasSession {
                    await beginConnection(interactive: false).value
                } else {
                    isStarting = false
                    if workspace != nil { logout() }
                    else if !Self.isUITesting { SiriVocabulary.update(nil) }
                    epoch = generation
                    if syncCache == nil {
                        let opened = try await openSyncCache()
                        try Task.checkCancellation()
                        guard generation == epoch else { return }
                        syncCache = opened
                    }
                    guard generation == epoch, !connecting else { return }
                    let previous = cacheCleanup
                    let cleanup = Task { [self] in
                        _ = await previous?.result
                        try await syncCache?.forget(connection: SyncCache.connectionKey(api.settings))
                        await notifications.revokeRegistration().value
                    }
                    cacheCleanup = cleanup
                    try await cleanup.value
                }
            } catch { if generation == epoch { self.error = error.localizedDescription } }
            return
        }
        await notifications.resume()
        await deliverDestination()
    }

    func prepareForIncomingCall() async -> WorkspaceStore? {
        if let connection { await connection.task.value }
        else if workspace == nil { await resume() }
        return workspace
    }

    func prepareForShortcut() async throws -> WorkspaceStore {
        if let connection { await connection.task.value }
        else { await resume() }
        try Task.checkCancellation()
        guard validatedSession, let workspace else {
            throw ZooError.auth("Zooを開いてログインと接続を確認してください。")
        }
        return workspace
    }

    func deliverDestination() async {
        guard let target = notifications.destination, let workspace, let userID else { return }
        guard target.userID.lowercased() == userID.lowercased() else {
            notifications.destination = nil
            return
        }
        let epoch = generation
        let delivery: (id: UUID, target: PushDestination, task: Task<Bool, Never>)
        if let current = destinationDelivery, current.target == target {
            delivery = current
        } else {
            destinationDelivery?.task.cancel()
            delivery = (UUID(), target, Task { await workspace.open(target) })
            destinationDelivery = delivery
        }
        let opened = await delivery.task.value
        if destinationDelivery?.id == delivery.id { destinationDelivery = nil }
        if opened, epoch == generation, notifications.destination == target {
            notifications.destination = nil
        }
    }
}
