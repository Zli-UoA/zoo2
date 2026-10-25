import SwiftUI

@main
struct ZooApp: App {
    @State private var session = SessionController()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(session)
                .tint(.zooAccent)
                .task {
                    if ProcessInfo.processInfo.arguments.contains("--demo") { session.loadDemo() }
                    else if !ProcessInfo.processInfo.arguments.contains("--uitesting") { await session.connect(interactive: false) }
                }
                .onChange(of: scenePhase) { _, phase in
                    if phase == .background { session.setActive(false) }
                    if phase == .active { session.setActive(true) }
                }
        }
    }
}
