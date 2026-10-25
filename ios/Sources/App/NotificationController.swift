import Foundation
import Observation
import UIKit
import UserNotifications

struct PushDestination: Equatable, Sendable {
    let userID: String
    let channelID: String
    let messageID: String?
    init?(payload: [AnyHashable: Any]) {
        guard let user = payload["user_id"] as? String, UUID(uuidString: user) != nil,
              let channel = payload["channel_id"] as? String, UUID(uuidString: channel) != nil else { return nil }
        let message = payload["message_id"] as? String
        guard message == nil || UUID(uuidString: message!) != nil else { return nil }
        userID = user; channelID = channel; messageID = message
    }
}

@Observable @MainActor
final class NotificationController {
    private(set) var status = "通知は無効です"
    private(set) var enabled = false
    var destination: PushDestination?
    private let registration = PushRegistration()
    private let transport: HTTPTransport
    private var api: APIClient?
    private var userID: String?
    private var account: String?
    private var token: String?
    private var generation = UUID()
    private let environment = Bundle.main.object(forInfoDictionaryKey: "APNSEnvironment") as? String == "production" ? "production" : "sandbox"
    init(transport: HTTPTransport = HTTPTransport()) { self.transport = transport }

    func connect(api: APIClient, userID: String) {
        generation = UUID()
        self.api = api; self.userID = userID
        account = PushReceipt.digest(api.settings.storageKey + "|" + userID)
        enabled = UserDefaults.standard.bool(forKey: preferenceKey)
        Task { await resume() }
    }
    private var preferenceKey: String { "zoo.push.enabled." + (account ?? "") }
    func resume() async {
        let epoch = generation
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        guard epoch == generation else { return }
        let permitted = settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional
        if enabled && permitted && api != nil { UIApplication.shared.registerForRemoteNotifications() }
        else { synchronize(allowRegistration: false) }
    }
    func enable() async {
        guard api != nil else { return }
        generation = UUID()
        let epoch = generation
        do {
            let allowed = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
            guard epoch == generation else { return }
            guard allowed else { status = "iOSの設定でZooの通知を許可してください"; return }
            enabled = true
            UserDefaults.standard.set(true, forKey: preferenceKey)
            status = "通知を登録しています"
            UIApplication.shared.registerForRemoteNotifications()
        } catch { if epoch == generation { status = error.localizedDescription } }
    }
    func disable() {
        generation = UUID()
        enabled = false
        UserDefaults.standard.set(false, forKey: preferenceKey)
        UIApplication.shared.unregisterForRemoteNotifications()
        synchronize(allowRegistration: false)
    }
    func received(token: Data) {
        self.token = token.map { String(format: "%02x", $0) }.joined()
        synchronize(allowRegistration: enabled)
    }
    func failed() { status = "通知を登録できませんでした。実機で接続を確認し、再試行してください。" }
    func accepts(_ target: PushDestination) -> Bool { enabled && userID == target.userID }
    func opened(_ target: PushDestination) { destination = target }
    func logout() {
        generation = UUID()
        UNUserNotificationCenter.current().removeAllDeliveredNotifications()
        api = nil; account = nil; userID = nil; destination = nil; enabled = false
        UIApplication.shared.unregisterForRemoteNotifications()
        synchronize(allowRegistration: false)
    }
    func cleanupAtLaunch() { synchronize(allowRegistration: false) }
    private func synchronize(allowRegistration: Bool) {
        let epoch = generation
        let desired = allowRegistration ? account : nil
        let backend = LivePushBackend(api: api, transport: transport)
        registration.update(backend: backend, account: desired, token: token, environment: environment) { [weak self] error in
            guard let self, epoch == self.generation else { return }
            if let error { self.status = "通知設定を同期できませんでした。次の接続時に再試行します。\(error.localizedDescription)" }
            else { self.status = desired == nil ? "通知は無効です" : (self.token == nil ? "通知を登録しています" : "通知は有効です") }
        }
    }
}

@MainActor
final class NotificationDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    var controller: NotificationController?
    private var pending: PushDestination?
    func bind(_ controller: NotificationController) {
        self.controller = controller
        if let pending { controller.opened(pending); self.pending = nil }
    }
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) { controller?.received(token: deviceToken) }
    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) { controller?.failed() }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        guard let target = PushDestination(payload: notification.request.content.userInfo) else { return [] }
        return await MainActor.run { controller?.accepts(target) == true ? [.banner, .sound] : [] }
    }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let target = PushDestination(payload: response.notification.request.content.userInfo) else { return }
        await MainActor.run {
            if let controller { controller.opened(target) } else { pending = target }
        }
    }
}
