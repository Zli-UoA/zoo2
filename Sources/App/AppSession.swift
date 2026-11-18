import Foundation
import Observation

@Observable @MainActor
final class AppSession {
    private(set) var userID: String?
    private(set) var workspace: WorkspaceStore?
    var connecting: Bool { connection != nil }
    private(set) var deviceAuthorization: DeviceAuthorizationPrompt?
    var error: String?
    let notifications: NotificationController
    private let authentication: Authentication
    private let api: APIClient
    private var connection: (task: Task<Void, Never>, interactive: Bool)?
    private var generation = UUID()

    init(settings: ConnectionSettings = ConnectionSettings(), transport: HTTPTransport = HTTPTransport(),
         notificationStore: any PushReceiptStore = KeychainPushReceiptStore(),
         previewKeys: any NotificationPreviewKeyStorage = NotificationPreviewKeyStore()) {
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
            session.workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
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
            if interactive {
                try await authentication.login { prompt in deviceAuthorization = prompt }
                deviceAuthorization = nil
            }
            try Task.checkCancellation()
            let user: Person = try await api.get("api/users/me")
            try Task.checkCancellation()
            guard generation == epoch else { return }
            userID = user.id
            let workspace = WorkspaceStore(client: NativeChatClient(api: api), user: user)
            workspace.onExpiredSession = { [weak self] in
                self?.logout()
                self?.error = ZooError.expiredSession.localizedDescription
            }
            self.workspace = workspace
            notifications.connect(api: api, userID: user.id)
            await notifications.resume()
            await deliverDestination()
        } catch {
            guard generation == epoch, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error {
                authentication.logout()
                notifications.logout()
            }
            self.error = error.localizedDescription
        }
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
        workspace?.shutdown()
        workspace = nil
        userID = nil
        notifications.logout()
        authentication.logout()
    }

    func resume() async {
        if userID == nil {
            guard !connecting else { return }
            do {
                if try authentication.hasSession {
                    await beginConnection(interactive: false).value
                } else {
                    await notifications.revokeRegistration().value
                }
            } catch { self.error = error.localizedDescription }
            return
        }
        await notifications.resume()
        await deliverDestination()
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
