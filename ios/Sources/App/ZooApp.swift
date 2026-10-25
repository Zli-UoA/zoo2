import SwiftUI

@main
struct ZooApp: App {
    @UIApplicationDelegateAdaptor(NotificationDelegate.self) private var delegate
    @State private var session = SessionController()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(session)
                .tint(.zooAccent)
                .preferredColorScheme(ProcessInfo.processInfo.arguments.contains("--uitesting") &&
                    ProcessInfo.processInfo.arguments.contains("--dark-appearance") ? .dark : nil)
                .task {
                    delegate.bind(session.notifications)
                    session.notifications.cleanupAtLaunch()
                    if ProcessInfo.processInfo.arguments.contains("--demo") { session.loadDemo() }
                    else if !ProcessInfo.processInfo.arguments.contains("--uitesting") { await session.connect(interactive: false) }
                }
                .onChange(of: scenePhase) { _, phase in
                    if phase == .background { session.setActive(false) }
                    if phase == .active {
                        session.setActive(true)
                        Task { await session.notifications.resume() }
                    }
                }
        }
    }
}
