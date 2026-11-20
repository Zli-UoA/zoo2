import Foundation
import Observation

@Observable @MainActor
final class AppSession {
    static let shared = configured()
    private(set) var userID: String?
    private(set) var workspace: WorkspaceStore?
    var connecting: Bool { connection != nil }
    private(set) var deviceAuthorization: DeviceAuthorizationPrompt?
    var error: String?
    let notifications: NotificationController
    @ObservationIgnored weak var systemCalls: SystemCallController?
    private let authentication: Authentication
    private let api: APIClient
    private var connection: (task: Task<Void, Never>, interactive: Bool)?
    private var generation = UUID()
    private var syncCache: SyncCache?
    private var syncSession: SyncSession?
    private var cacheCleanup: Task<Void, Error>?
    private var validatedSession = false

    init(settings: ConnectionSettings = ConnectionSettings(), transport: HTTPTransport = HTTPTransport(),
         notificationStore: any PushReceiptStore = KeychainPushReceiptStore(),
         previewKeys: any NotificationPreviewKeyStorage = NotificationPreviewKeyStore(), syncCache: SyncCache? = nil) {
        self.syncCache = syncCache
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
            session.userID = client.currentUser.id
            session.validatedSession = true
            let calls = CallStore(client: client, userID: client.currentUser.id, makeMedia: { PreviewCallMedia() }, permission: { true })
            session.workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil, calls: calls)
            if let workspace = session.workspace, ProcessInfo.processInfo.arguments.contains("--uitesting-attachments") {
                Task { await client.prepareAttachmentDraft(in: workspace) }
            }
            if let workspace = session.workspace {
                client.runShortcutPreview(in: workspace)
            }
            return session
        }
        if isUITesting {
            return AppSession(settings: ConnectionSettings(server: "https://ui.test.invalid", clientID: "ui-test"),
                              notificationStore: PreviewPushReceiptStore(), previewKeys: InMemoryNotificationPreviewKeys())
        }
        #endif
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

    func startLogin() {
        guard !connecting, userID == nil else { return }
        notifications.discardAccountNotifications()
        beginConnection(interactive: true)
    }

    @discardableResult
    private func beginConnection(interactive: Bool) -> Task<Void, Never> {
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
            try await cacheCleanup?.value
            cacheCleanup = nil
            if syncCache == nil { syncCache = try await SyncCache.open(inMemory: Self.isUITesting) }
            guard let syncCache else { throw ZooError.invalidResponse }
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
            } else if isOfflineFailure(error), workspace != nil {
                workspace?.error = "オフラインです。保存済みのデータを表示しています。"
                return
            }
            self.error = error.localizedDescription
        }
    }

    private func installWorkspace(_ user: Person, cache: SyncCache, epoch: UUID) async throws {
        let lease = try await cache.activate(connection: SyncCache.connectionKey(api.settings), user: user)
        guard generation == epoch, !Task.isCancelled else {
            try await cache.revoke(lease)
            throw CancellationError()
        }
        workspace?.shutdown()
        let sync = SyncSession(cache: cache, lease: lease)
        syncSession = sync
        userID = user.id
        let workspace = WorkspaceStore(client: NativeChatClient(api: api, sync: sync), user: user)
        sync.onChannelRevoked = { [weak workspace] id in workspace?.inbox.revokeChannel(id) }
        workspace.onExpiredSession = { [weak self] in
            self?.logout()
            self?.error = ZooError.expiredSession.localizedDescription
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
        deviceAuthorization = nil
        error = nil
    }

    func logout() {
        cancelConnection()
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
        authentication.logout()
        if let syncSession { cacheCleanup = syncSession.close() }
        self.syncSession = nil
    }

    func resume() async {
        if userID == nil || !validatedSession {
            guard !connecting else { return }
            do {
                if try authentication.hasSession {
                    await beginConnection(interactive: false).value
                } else {
                    if workspace != nil { logout() }
                    if syncCache == nil { syncCache = try await SyncCache.open(inMemory: Self.isUITesting) }
                    try await syncCache?.forget(connection: SyncCache.connectionKey(api.settings))
                    await notifications.revokeRegistration().value
                }
            } catch { self.error = error.localizedDescription }
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
        if await workspace.open(target), epoch == generation, notifications.destination == target {
            notifications.destination = nil
        }
    }
}
