import Foundation
import Observation
import UIKit
import UserNotifications

struct PushDestination: Equatable, Sendable {
    let userID: String
    let channelID: String
    let messageID: String?
    init?(payload: [AnyHashable: Any]) {
        guard payload["type"] as? String != "push_test",
              let user = payload["user_id"] as? String, UUID(uuidString: user) != nil,
              let channel = payload["channel_id"] as? String, UUID(uuidString: channel) != nil else { return nil }
        let message = payload["message_id"] as? String
        guard message == nil || UUID(uuidString: message!) != nil else { return nil }
        userID = user; channelID = channel; messageID = message
    }
}

enum PushNotice: Equatable, Sendable {
    case message(PushDestination)
    case test(userID: String, id: UUID)

    init?(payload: [AnyHashable: Any]) {
        if payload["type"] as? String == "push_test" {
            guard let user = payload["user_id"] as? String, UUID(uuidString: user) != nil,
                  let id = payload["test_id"] as? String, let testID = UUID(uuidString: id) else { return nil }
            self = .test(userID: user, id: testID)
        } else {
            guard let destination = PushDestination(payload: payload) else { return nil }
            self = .message(destination)
        }
    }

    var userID: String {
        switch self {
        case .message(let destination): destination.userID
        case .test(let userID, _): userID
        }
    }
}

@MainActor
protocol NotificationPreviewKeyStorage {
    func load() throws -> NotificationPreviewKey?
    func save(_ key: NotificationPreviewKey?) throws
}

extension NotificationPreviewKeyStore: NotificationPreviewKeyStorage {}

@Observable @MainActor
final class NotificationController {
    private(set) var status = "通知は無効です"
    private(set) var enabled = false
    private(set) var registrationFailed = false
    private(set) var sendingTest = false
    var testStatus: String? { testAttempt?.received == true ? "この端末でテスト通知を受信しました。" : testFeedback }
    var destination: PushDestination?
    private let registration: PushRegistration
    private let voipRegistration: PushRegistration
    private var voipToken: String?
    private let receiptStore: any PushReceiptStore
    private let previewKeys: any NotificationPreviewKeyStorage
    private let transport: HTTPTransport
    private var api: APIClient?
    private var userID: String?
    private var account: String?
    private var token: String?
    private var authorizationDenied = false
    private var registrationGeneration = UUID()
    private var registeredReceipt: PushReceipt?
    private struct TestAttempt: Codable {
        let id: UUID
        var received = false
    }
    private var testAttempt: TestAttempt?
    private var testFeedback: String?
    private var pendingTest: PushNotice?
    private var testPreferenceKey: String { "zoo.push.test." + (account ?? "") }
    private var generation = UUID()
    private let environment = Bundle.main.object(forInfoDictionaryKey: "APNSEnvironment") as? String == "production" ? "production" : "sandbox"
    init(transport: HTTPTransport, receiptStore: any PushReceiptStore, previewKeys: any NotificationPreviewKeyStorage,
         voipStore: any PushReceiptStore = KeychainPushReceiptStore(service: "works.zli.zoo.voip")) {
        self.transport = transport
        self.receiptStore = receiptStore
        self.previewKeys = previewKeys
        registration = PushRegistration(store: receiptStore)
        voipRegistration = PushRegistration(store: voipStore, kind: .voip)
    }

    func connect(api: APIClient, userID: String) {
        generation = UUID()
        registeredReceipt = nil
        testFeedback = nil
        self.api = api; self.userID = userID
        account = PushReceipt.digest(api.settings.storageKey + "|" + userID)
        testAttempt = UserDefaults.standard.data(forKey: testPreferenceKey).flatMap { try? JSONDecoder().decode(TestAttempt.self, from: $0) }
        enabled = UserDefaults.standard.bool(forKey: preferenceKey)
        validateStoredPreviewKey()
        if let pendingTest { _ = receive(pendingTest); self.pendingTest = nil }
    }
    private var preferenceKey: String { "zoo.push.enabled." + (account ?? "") }
    func resume(authorization: @MainActor () async -> UNAuthorizationStatus = {
        await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    }) async {
        guard api != nil else { return }
        let epoch = generation
        let authorizationStatus = await authorization()
        guard epoch == generation else { return }
        let permitted = authorizationStatus == .authorized || authorizationStatus == .provisional
        authorizationDenied = !permitted
        if enabled && permitted { UIApplication.shared.registerForRemoteNotifications() }
        else { revokeRegistration() }
        await checkDeliveredTest()
    }
    func enable() async {
        guard api != nil else { return }
        generation = UUID()
        let epoch = generation
        do {
            let allowed = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
            guard epoch == generation else { return }
            authorizationDenied = !allowed
            guard allowed else {
                revokeRegistration()
                return
            }
            registrationFailed = false
            enabled = true
            UserDefaults.standard.set(true, forKey: preferenceKey)
            status = "通知を登録しています"
            UIApplication.shared.registerForRemoteNotifications()
        } catch { if epoch == generation { status = error.localizedDescription } }
    }
    func disable() {
        generation = UUID()
        registeredReceipt = nil
        saveTestAttempt(nil)
        testFeedback = nil
        enabled = false
        UserDefaults.standard.set(false, forKey: preferenceKey)
        UIApplication.shared.unregisterForRemoteNotifications()
        revokeRegistration()
    }
    @discardableResult
    func received(token: Data) -> Task<Void, Never>? {
        let newToken = token.map { String(format: "%02x", $0) }.joined()
        if self.token != newToken {
            registrationGeneration = UUID()
            registeredReceipt = nil
            // 起動時の初回コールバックでは、同じ端末登録で公開済みの鍵を維持する。
            if self.token != nil || (try? receiptStore.load())?.token != newToken {
                removePreviewKey()
            }
            self.token = newToken
        }
        guard api != nil else { return nil }
        return synchronize(allowRegistration: enabled && !authorizationDenied)
    }
    func failed() {
        registeredReceipt = nil
        registrationFailed = true
        status = "通知を登録できませんでした。実機で接続を確認し、再試行してください。"
    }
    @discardableResult
    func receivedVoIPToken(_ data: Data?) -> Task<Void, Never>? {
        voipToken = data?.map { String(format: "%02x", $0) }.joined()
        guard api != nil else { return nil }
        return synchronize(allowRegistration: enabled && !authorizationDenied)
    }
    private func saveTestAttempt(_ attempt: TestAttempt?) {
        testAttempt = attempt
        guard account != nil else { return }
        if let attempt, let data = try? JSONEncoder().encode(attempt) {
            UserDefaults.standard.set(data, forKey: testPreferenceKey)
        } else {
            UserDefaults.standard.removeObject(forKey: testPreferenceKey)
        }
    }

    func receive(_ notice: PushNotice) -> Bool {
        if userID == nil, case .test = notice {
            pendingTest = notice
            return false
        }
        guard enabled, userID?.caseInsensitiveCompare(notice.userID) == .orderedSame else { return false }
        if case .test(_, let id) = notice, id == testAttempt?.id {
            saveTestAttempt(TestAttempt(id: id, received: true))
        }
        return true
    }

    func checkDeliveredTest(using delivered: @MainActor () async -> [PushNotice] = {
        let notifications = await UNUserNotificationCenter.current().deliveredNotifications()
        return notifications.compactMap { notification in
            let info = notification.request.content.userInfo
            guard info["type"] as? String == "push_test" else { return nil }
            return PushNotice(payload: info)
        }
    }) async {
        guard enabled, let attempt = testAttempt, !attempt.received else { return }
        let epoch = generation
        let notices = await delivered()
        guard epoch == generation, testAttempt?.id == attempt.id else { return }
        for case let .test(recipient, id) in notices where id == attempt.id {
            if receive(.test(userID: recipient, id: id)) { return }
        }
    }

    var canSendTest: Bool {
        guard enabled, let registeredReceipt else { return false }
        return registeredReceipt.account == account && registeredReceipt.token == token
    }

    func sendTest() async {
        guard canSendTest, !sendingTest, let api, let account, let registeredReceipt else { return }
        let epoch = generation
        let id = UUID()
        sendingTest = true
        saveTestAttempt(TestAttempt(id: id))
        testFeedback = nil
        defer { sendingTest = false }
        do {
            try await LivePushBackend(api: api, transport: transport).sendTest(to: registeredReceipt, account: account, id: id)
            if epoch == generation { testFeedback = "テスト通知を送信しました。" }
        } catch {
            if epoch == generation { testFeedback = "テスト通知を送信できませんでした。\(error.localizedDescription)" }
        }
    }
    func opened(_ notice: PushNotice) {
        switch notice {
        case .message(let target): destination = target
        case .test: _ = receive(notice)
        }
    }
    func logout() {
        generation = UUID()
        registeredReceipt = nil
        saveTestAttempt(nil)
        testFeedback = nil
        pendingTest = nil
        api = nil; account = nil; userID = nil; destination = nil; enabled = false
        UIApplication.shared.unregisterForRemoteNotifications()
        discardAccountNotifications()
    }

    @discardableResult
    func discardAccountNotifications(removeDelivered: () -> Void = {
        UNUserNotificationCenter.current().removeAllDeliveredNotifications()
    }) -> Task<Void, Never> {
        let revocation = revokeRegistration()
        // 配信済み本文は購読の解除後にも残るため、端末登録の有無を問わず削除する。
        removeDelivered()
        return revocation
    }

    @discardableResult
    func revokeRegistration() -> Task<Void, Never> { synchronize(allowRegistration: false) }

    private func validateStoredPreviewKey() {
        if enabled, !authorizationDenied, let userID, let receipt = try? receiptStore.load(),
           receipt.server == api?.settings.server, receipt.account == account, receipt.environment == environment,
           receipt.revocationPending != true, token == nil || receipt.token == token,
           (try? previewKeys.load()) == NotificationPreviewKey(userID: userID, secret: receipt.secret) {
            return
        }
        removePreviewKey()
    }

    @discardableResult
    private func removePreviewKey() -> Bool {
        do { try previewKeys.save(nil); return true }
        catch { reportSyncError(error); return false }
    }

    private func reportSyncError(_ error: any Error) {
        registrationFailed = true
        status = "通知設定を同期できませんでした。次の接続時に再試行します。\(error.localizedDescription)"
    }

    @discardableResult
    private func synchronize(allowRegistration: Bool) -> Task<Void, Never> {
        if !allowRegistration {
            registrationGeneration = UUID()
            registeredReceipt = nil
        }
        let keyCleared = allowRegistration || removePreviewKey()
        let revision = registrationGeneration
        let epoch = generation
        let desired = allowRegistration ? account : nil
        let backend = LivePushBackend(api: api, transport: transport)
        let standard = registration.update(backend: backend, account: desired, token: token, environment: environment) { [weak self] result in
            guard let self, epoch == self.generation, revision == self.registrationGeneration else { return }
            do {
                let receipt = try result.get()
                if let receipt, let userID = self.userID {
                    let key = NotificationPreviewKey(userID: userID, secret: receipt.secret)
                    if (try? self.previewKeys.load()) != key { try self.previewKeys.save(key) }
                }
                self.registeredReceipt = receipt
                guard keyCleared else { return }
                self.registrationFailed = false
                if self.authorizationDenied && self.api != nil {
                    self.status = "iOSの設定でZooの通知を許可してください"
                } else {
                    self.status = desired == nil ? "通知は無効です" : (receipt == nil ? "通知を登録しています" : "通知は有効です")
                }
            } catch {
                self.registeredReceipt = nil
                self.reportSyncError(error)
            }
        }
        var voipError: Error?
        let voip = voipRegistration.update(backend: backend, account: voipToken == nil ? nil : desired,
                                           token: voipToken, environment: environment) { result in
            if case .failure(let error) = result { voipError = error }
        }
        return Task {
            await standard.value; await voip.value
            guard epoch == generation, revision == registrationGeneration else { return }
            if let voipError { reportSyncError(voipError) }
        }
    }
}

@MainActor
final class NotificationDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    let session = AppSession.configured()
    var controller: NotificationController?
    let systemCalls = SystemCallController()
    private var pendingMessageTap: PushDestination?
    private var pendingTest: PushNotice?
    func bind(_ controller: NotificationController) {
        self.controller = controller
        if let pendingMessageTap { controller.opened(.message(pendingMessageTap)); self.pendingMessageTap = nil }
        if let pendingTest { _ = controller.receive(pendingTest); self.pendingTest = nil }
    }
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        if !AppSession.isUITesting {
            bind(session.notifications)
            systemCalls.bind(session)
            systemCalls.setForeground(application.applicationState == .active)
            systemCalls.start()
        }
        return true
    }
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) { controller?.received(token: deviceToken) }
    func application(_ application: UIApplication, supportedInterfaceOrientationsFor window: UIWindow?) -> UIInterfaceOrientationMask {
        AppOrientation.supportedOrientations(in: window?.windowScene)
    }
    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) { controller?.failed() }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        guard let notice = PushNotice(payload: notification.request.content.userInfo) else { return [] }
        return await presentationOptions(for: notice)
    }
    func presentationOptions(for notice: PushNotice) -> UNNotificationPresentationOptions {
        guard let controller else {
            if case .test = notice { pendingTest = notice }
            return []
        }
        return controller.receive(notice) ? [.banner, .sound] : []
    }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let target = PushNotice(payload: response.notification.request.content.userInfo) else { return }
        await MainActor.run {
            if let controller { controller.opened(target) }
            else {
                switch target {
                case .message(let destination): pendingMessageTap = destination
                case .test: pendingTest = target
                }
            }
        }
    }
}
