import UIKit
import UserNotifications
import Intents

@MainActor
final class NotificationDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    let session = AppSession.shared
    var controller: NotificationController?
    let systemCalls = SystemCallController()
    private var watchCompanion: WatchCompanionController?
    private var pendingMessageTap: PushDestination?
    private var pendingTest: PushNotice?
    func application(_ application: UIApplication, handlerFor intent: INIntent) -> Any? {
        intent is INSendMessageIntent ? SiriMessageHandler() : nil
    }
    func bind(_ controller: NotificationController) {
        self.controller = controller
        if let pendingMessageTap { controller.opened(.message(pendingMessageTap)); self.pendingMessageTap = nil }
        if let pendingTest { _ = controller.receive(pendingTest); self.pendingTest = nil }
    }
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        UNUserNotificationCenter.current().setNotificationCategories([IncomingCallNotifications.category])
        if !AppSession.isUITesting {
            ZooShortcuts.updateAppShortcutParameters()
            bind(session.notifications)
            systemCalls.bind(session)
            systemCalls.setForeground(application.applicationState == .active)
            systemCalls.start()
            watchCompanion = WatchCompanionController(session: session, calls: systemCalls)
            watchCompanion?.start()
        }
        return true
    }
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) { controller?.received(token: deviceToken) }
    func application(_ application: UIApplication, supportedInterfaceOrientationsFor window: UIWindow?) -> UIInterfaceOrientationMask {
        AppOrientation.supportedOrientations(in: window?.windowScene)
    }
    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) { controller?.failed() }
    // async版の自動生成ブリッジは完了時にメインスレッドを離れるため、iOSへの完了通知まで隔離する。
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                           withCompletionHandler completionHandler: @escaping @Sendable (UNNotificationPresentationOptions) -> Void) {
        let notice = PushNotice(payload: notification.request.content.userInfo)
        Task { @MainActor in
            completionHandler(notice.map { presentationOptions(for: $0) } ?? [])
        }
    }
    func presentationOptions(for notice: PushNotice) -> UNNotificationPresentationOptions {
        guard let controller else {
            if case .test = notice { pendingTest = notice }
            return []
        }
        return controller.receive(notice) ? [.banner, .sound] : []
    }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                           withCompletionHandler completionHandler: @escaping @Sendable () -> Void) {
        let target = PushNotice(payload: response.notification.request.content.userInfo)
        let callAction = IncomingCallNotificationAction(request: response.notification.request,
                                                        actionIdentifier: response.actionIdentifier)
        Task { @MainActor in
            defer { completionHandler() }
            if let callAction {
                await systemCalls.performNotificationAction(callAction)
                return
            }
            guard let target else { return }
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
