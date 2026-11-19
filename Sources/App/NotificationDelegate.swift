import UIKit
import UserNotifications

@MainActor
final class NotificationDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    let session = AppSession.shared
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
            ZooShortcuts.updateAppShortcutParameters()
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
