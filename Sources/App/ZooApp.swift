import SwiftUI
import WebKit

@main
struct ZooApp: App {
    @UIApplicationDelegateAdaptor(NotificationDelegate.self) private var delegate
    @State private var session = WebSession()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView(session: session)
                .tint(Color(red: 0.15, green: 0.39, blue: 0.35))
                .task {
                    delegate.bind(session.notifications)
                    session.notifications.cleanupAtLaunch()
                    if !ProcessInfo.processInfo.arguments.contains("--uitesting") {
                        await session.connect(interactive: false)
                    }
                }
                .onChange(of: scenePhase) { _, value in
                    if value == .active { Task { await session.resume() } }
                }
                .onChange(of: session.notifications.status) { _, _ in
                    Task { await session.publishNotificationState() }
                }
                .onChange(of: session.notifications.destination) { _, _ in
                    Task { await session.deliverDestination() }
                }
        }
    }
}

private struct RootView: View {
    @Bindable var session: WebSession

    var body: some View {
        if let page = session.page {
            VStack(spacing: 0) {
                HStack {
                    Text("Zoo").font(.headline)
                    Spacer()
                    if page.isLoading { ProgressView().controlSize(.small) }
                    Button("再読み込み", systemImage: "arrow.clockwise") { session.loadWeb() }
                        .labelStyle(.iconOnly)
                    Button("ログアウト", systemImage: "rectangle.portrait.and.arrow.right") { session.logout() }
                        .labelStyle(.iconOnly)
                }
                .padding(.horizontal).padding(.vertical, 8)
                WebView(page)
            }
            .overlay(alignment: .top) {
                if let error = session.webError {
                    ContentUnavailableView {
                        Label("接続できませんでした", systemImage: "wifi.exclamationmark")
                    } description: { Text(error) }
                    actions: { Button("再読み込み") { session.loadWeb() } }
                    .background(.background)
                }
            }
        } else {
            VStack(spacing: 24) {
                Image(systemName: "pawprint.fill").font(.system(size: 64)).foregroundStyle(.tint)
                Text("ようこそ Zoo へ").font(.largeTitle.bold())
                Text("いつもの会話を、iPhone で。")
                if let error = session.error {
                    Text(error).font(.callout).foregroundStyle(.red).accessibilityIdentifier("login-error")
                }
                Button {
                    Task { await session.connect(interactive: true) }
                } label: {
                    if session.connecting { ProgressView() } else { Text("ログイン") }
                }
                .buttonStyle(.borderedProminent)
                .disabled(session.connecting)
                .accessibilityIdentifier("login")
            }.padding(24)
        }
    }
}
