import SwiftUI

@main
struct ZooApp: App {
    @UIApplicationDelegateAdaptor(NotificationDelegate.self) private var delegate
    private var session: AppSession { delegate.session }
    @Environment(\.scenePhase) private var scenePhase
    @AppStorage(AppearancePreference.key) private var appearance = AppearancePreference.system

    var body: some Scene {
        WindowGroup {
            RootView(session: session)
                .preferredColorScheme(session.workspace == nil && (session.isStarting || session.restoring)
                                      ? nil : appearance.colorScheme)
                .environment(\.locale, Locale(identifier: "ja_JP"))
                .tint(AppDesign.navigationTint)
                .task {
                    guard !AppSession.isUITesting else { return }
                    delegate.systemCalls.setForeground(scenePhase == .active)
                    await session.resume()
                }
                .onChange(of: scenePhase) { _, value in
                    if !AppSession.isUITesting { delegate.systemCalls.setForeground(value == .active) }
                    if value == .active, !session.isStarting, !AppSession.isUITesting { Task { await session.resume() } }
                }
                .onChange(of: session.notifications.destination) { _, _ in
                    Task { await session.deliverDestination() }
                }
        }
    }
}
