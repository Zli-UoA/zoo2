import Foundation
import Observation
import WebKit

@Observable @MainActor
final class WebSession {
    private(set) var page: WebPage?
    private(set) var userID: String?
    private(set) var connecting = false
    private(set) var webReady = false
    var error: String?
    private(set) var webError: String?
    private var navigationTask: Task<Void, Never>?
    let notifications: NotificationController
    private let authentication: Authentication
    private let api: APIClient
    private var bridge: WebBridge?
    private var generation = UUID()

    init() {
        let settings = ConnectionSettings()
        let transport = HTTPTransport()
        authentication = Authentication(settings: settings, transport: transport)
        api = APIClient(settings: settings, auth: authentication, transport: transport)
        notifications = NotificationController(transport: transport)
    }

    func connect(interactive: Bool) async {
        guard !connecting, userID == nil else { return }
        guard interactive || authentication.hasSession else { return }
        let epoch = generation
        connecting = true
        error = nil
        defer { if generation == epoch { connecting = false } }
        do {
            if interactive { try await authentication.login() }
            let user: Person = try await api.get("api/users/me")
            guard generation == epoch else { return }
            userID = user.id
            notifications.connect(api: api, userID: user.id)
            loadWeb()
        } catch {
            guard generation == epoch else { return }
            if case ZooError.expiredSession = error { authentication.logout() }
            self.error = error.localizedDescription
        }
    }

    func loadWeb() {
        bridge?.active = false
        let bridge = WebBridge()
        bridge.handle = { [weak self] command in
            guard let self else { throw CancellationError() }
            return try await self.handle(command)
        }
        var configuration = WebPage.Configuration()
        // A fresh, ephemeral store prevents a previous account's Web data surviving logout.
        configuration.websiteDataStore = .nonPersistent()
        configuration.applicationNameForUserAgent = "Zoo-iOS/1"
        configuration.userContentController.addScriptMessageHandler(bridge, contentWorld: .page, name: "zoo")
        configuration.userContentController.addUserScript(WKUserScript(source:
            "Object.defineProperty(window, '__ZOO_NATIVE__', { value: Object.freeze({version: 1}), writable: false, configurable: false });",
            injectionTime: .atDocumentStart, forMainFrameOnly: true))
        let page = WebPage(configuration: configuration, navigationDecider: WebNavigation())
        self.bridge = bridge
        self.page = page
        webReady = false
        webError = nil
        navigationTask?.cancel()
        navigationTask = Task { [weak self, weak page] in
            guard let page else { return }
            do {
                for try await _ in page.load(URLRequest(url: WebTrust.home, timeoutInterval: 30)) { }
                try await Task.sleep(for: .seconds(15))
                if !Task.isCancelled, self?.page === page, self?.webReady == false {
                    self?.webError = "アプリに対応した画面を利用できません。時間をおいて再度お試しください。"
                }
            } catch {
                guard !Task.isCancelled, self?.page === page else { return }
                self?.webError = "通信状態を確認して、再読み込みしてください。"
            }
        }
    }

    private func handle(_ command: BridgeCommand) async throws -> Any {
        guard let userID else { throw ZooError.expiredSession }
        let epoch = generation
        switch command {
        case .session:
            return ["version": 1, "userID": userID] as [String: Any]
        case .token:
            do {
                let token = try await authentication.accessToken()
                guard epoch == generation else { throw CancellationError() }
                return ["accessToken": token]
            } catch ZooError.expiredSession {
                logout()
                throw ZooError.expiredSession
            }
        case .logout:
            logout()
            return ["ok": true]
        case .ready:
            webError = nil
            webReady = true
            Task { await deliverDestination() }
            return ["ok": true]
        case .notificationState:
            return ["enabled": notifications.enabled, "status": notifications.status, "failed": notifications.registrationFailed] as [String: Any]
        case .enableNotifications:
            await notifications.enable()
            guard epoch == generation else { throw CancellationError() }
            return ["enabled": notifications.enabled, "status": notifications.status, "failed": notifications.registrationFailed] as [String: Any]
        case .disableNotifications:
            notifications.disable()
            return ["enabled": notifications.enabled, "status": notifications.status, "failed": notifications.registrationFailed] as [String: Any]
        }
    }

    func logout() {
        generation = UUID()
        bridge?.active = false
        bridge?.handle = nil
        bridge = nil
        navigationTask?.cancel()
        navigationTask = nil
        page = nil
        webReady = false
        userID = nil
        connecting = false
        notifications.logout()
        authentication.logout()
    }

    func publishNotificationState() async {
        guard let page, webReady, WebTrust.allows(page.url) else { return }
        _ = try? await page.callJavaScript("window.dispatchEvent(new Event('zoo:native:notifications'));")
    }

    func resume() async {
        await notifications.resume()
        if let page, webReady, WebTrust.allows(page.url) {
            _ = try? await page.callJavaScript("window.dispatchEvent(new Event('focus')); window.dispatchEvent(new Event('zoo:native:resume'));")
        }
        await deliverDestination()
    }

    func deliverDestination() async {
        guard let target = notifications.destination, let userID else { return }
        guard target.userID.lowercased() == userID.lowercased() else {
            notifications.destination = nil
            return
        }
        guard let page, webReady, WebTrust.allows(page.url) else { return }
        let epoch = generation
        var payload = ["userID": target.userID, "channelID": target.channelID]
        if let message = target.messageID { payload["messageID"] = message }
        do {
            let handled = try await page.callJavaScript(
                "return typeof window.__zooNavigate === 'function' ? await window.__zooNavigate(destination) : false;",
                arguments: ["destination": payload]) as? Bool
            if handled == true, epoch == generation, self.page === page, notifications.destination == target {
                notifications.destination = nil
            }
        } catch { /* Keep the target until the Web UI is ready again. */ }
    }
}
