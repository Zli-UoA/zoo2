import SwiftUI

@main
struct ZooWatchApp: App {
    @State private var session = WatchSession()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            WatchHomeView(state: session.state, send: session.send)
                .environment(\.locale, Locale(identifier: "ja_JP"))
                .task(id: scenePhase) {
                    session.setActive(scenePhase == .active)
                    guard scenePhase == .active else { return }
                    while !Task.isCancelled {
                        do { try await Task.sleep(for: .seconds(15)) } catch { return }
                        session.send(.refresh)
                    }
                }
        }
    }
}
