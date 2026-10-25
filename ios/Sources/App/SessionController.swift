import Foundation
import Observation

@Observable @MainActor
final class SessionController {
    private(set) var settings: ConnectionSettings
    private(set) var context: WorkspaceContext?
    private(set) var isDemo = false
    private(set) var isConnecting = false
    var error: String?
    let notifications: NotificationController
    private var authentication: Authentication
    private var repositorySession: RepositorySession
    private let transport: HTTPTransport
    private var generation = UUID()
    private var active = true

    init(settings configured: ConnectionSettings? = nil, transport: HTTPTransport = HTTPTransport()) {
        let saved = UserDefaults.standard.data(forKey: "zoo.connection")
        let settings = configured ?? saved.flatMap { try? JSONDecoder().decode(ConnectionSettings.self, from: $0) } ?? ConnectionSettings()
        self.settings = settings
        self.transport = transport
        notifications = NotificationController(transport: transport)
        let authentication = Authentication(settings: settings, transport: transport)
        self.authentication = authentication
        repositorySession = RepositorySession(api: APIClient(settings: settings, auth: authentication, transport: transport))
    }

    func saveSettings(_ value: ConnectionSettings) throws {
        try value.validate()
        logout()
        settings = value
        UserDefaults.standard.set(try JSONEncoder().encode(value), forKey: "zoo.connection")
        configureConnection()
    }

    private func configureConnection() {
        authentication = Authentication(settings: settings, transport: transport)
        repositorySession = RepositorySession(api: APIClient(settings: settings, auth: authentication, transport: transport))
    }

    func connect(interactive: Bool) async {
        guard !isConnecting, context == nil else { return }
        if !interactive && !authentication.hasSession { return }
        isConnecting = true
        error = nil
        let epoch = generation
        defer { if epoch == generation { isConnecting = false } }
        do {
            if interactive { try await authentication.login() }
            let user: Person = try await repositorySession.api.get("api/users/me")
            guard epoch == generation else { return }
            let session = repositorySession
            let repository = LiveWorkspaceRepository(session: session)
            repository.state.me = user
            notifications.connect(api: session.api, userID: user.id)
            install(WorkspaceContext(repository: repository) {
                LiveMessageRepository(channelID: $0, userID: user.id, session: session)
            })
        } catch { if epoch == generation { report(error) } }
    }

    func loadDemo() {
        logout()
        isDemo = true
        let repository = DemoWorkspaceRepository()
        install(WorkspaceContext(repository: repository) { repository.makeMessages(channelID: $0) })
    }

    private func install(_ context: WorkspaceContext) {
        context.onSessionExpired = { [weak self] in self?.report(ZooError.expiredSession) }
        self.context = context
        context.setActive(active)
    }

    func setActive(_ value: Bool) { active = value; context?.setActive(value) }
    func reconnect() { context?.reconnect() }

    func report(_ problem: Error) {
        if problem is CancellationError { return }
        if case ZooError.expiredSession = problem { logout() }
        error = problem.localizedDescription
    }

    func perform(_ operation: () async throws -> Void) async -> Bool {
        let epoch = generation
        do { try await operation(); return epoch == generation }
        catch { if epoch == generation { report(error) }; return false }
    }

    func logout() {
        notifications.logout()
        generation = UUID()
        repositorySession.invalidate()
        authentication.logout()
        context?.reset()
        context = nil
        isDemo = false; isConnecting = false; error = nil
        configureConnection()
    }
}
