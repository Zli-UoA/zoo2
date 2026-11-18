import Foundation
import Observation

@Observable @MainActor
final class AppSession {
    private(set) var userID: String?
    private(set) var workspace: WorkspaceStore?
    private(set) var connecting = false
    private(set) var deviceAuthorization: DeviceAuthorizationPrompt?
    var error: String?
    let notifications: NotificationController
    private let authentication: Authentication
    private let api: APIClient
    private var connectionTask: Task<Void, Never>?
    private var generation = UUID()

    init(settings: ConnectionSettings = ConnectionSettings(), transport: HTTPTransport = HTTPTransport(),
         notificationStore: any PushReceiptStore = KeychainPushReceiptStore()) {
        authentication = Authentication(settings: settings, transport: transport)
        api = APIClient(settings: settings, auth: authentication, transport: transport)
        notifications = NotificationController(transport: transport, receiptStore: notificationStore)
    }

    static func configured() -> AppSession {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--uitesting-workspace") {
            let session = AppSession(settings: ConnectionSettings(server: "https://ui.test.invalid", clientID: "ui-test"), notificationStore: PreviewPushReceiptStore())
            let client = PreviewChatClient()
            session.userID = client.currentUser.id
            session.workspace = WorkspaceStore(client: client, user: client.currentUser)
            return session
        }
        if isUITesting {
            return AppSession(settings: ConnectionSettings(server: "https://ui.test.invalid", clientID: "ui-test"), notificationStore: PreviewPushReceiptStore())
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
        guard connectionTask == nil, !connecting, userID == nil else { return }
        let epoch = generation
        connectionTask = Task {
            guard generation == epoch, !Task.isCancelled else { return }
            await connect(interactive: true)
        }
    }

    func cancelLogin() {
        guard connectionTask != nil || connecting else { return }
        generation = UUID()
        connectionTask?.cancel()
        connectionTask = nil
        authentication.logout()
        deviceAuthorization = nil
        connecting = false
        error = nil
    }

    func connect(interactive: Bool) async {
        guard !connecting, userID == nil, interactive || authentication.hasSession else { return }
        let epoch = generation
        connecting = true
        error = nil
        defer {
            if generation == epoch {
                connecting = false
                deviceAuthorization = nil
                connectionTask = nil
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
            await deliverDestination()
        } catch {
            guard generation == epoch, !Task.isCancelled, !(error is CancellationError) else { return }
            if case ZooError.expiredSession = error { authentication.logout() }
            self.error = error.localizedDescription
        }
    }

    func logout() {
        generation = UUID()
        connectionTask?.cancel()
        connectionTask = nil
        workspace?.shutdown()
        workspace = nil
        deviceAuthorization = nil
        userID = nil
        connecting = false
        error = nil
        notifications.logout()
        authentication.logout()
    }

    func resume() async {
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
