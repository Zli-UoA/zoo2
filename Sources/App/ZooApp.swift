import SwiftUI
import SafariServices

@main
struct ZooApp: App {
    @UIApplicationDelegateAdaptor(NotificationDelegate.self) private var delegate
    @State private var session = AppSession.configured()
    @Environment(\.scenePhase) private var scenePhase
    @AppStorage(AppearancePreference.key) private var appearance = AppearancePreference.system

    var body: some Scene {
        WindowGroup {
            RootView(session: session)
                .preferredColorScheme(appearance.colorScheme)
                .environment(\.locale, Locale(identifier: "ja_JP"))
                .tint(AppDesign.tint)
                .task {
                    guard !AppSession.isUITesting else { return }
                    delegate.bind(session.notifications)
                    await session.resume()
                }
                .onChange(of: scenePhase) { _, value in
                    if value == .active, !AppSession.isUITesting { Task { await session.resume() } }
                }
                .onChange(of: session.notifications.destination) { _, _ in
                    Task { await session.deliverDestination() }
                }
        }
    }
}

private struct RootView: View {
    @Bindable var session: AppSession
    @State private var showingVerification = false

    var body: some View {
        Group {
            if let workspace = session.workspace {
                WorkspaceView(workspace: workspace, session: session)
            } else {
                ScrollView {
                    VStack(spacing: 24) {
                        Image(systemName: "pawprint.fill").font(.system(size: 64)).foregroundStyle(.tint)
                        Text("ようこそ Zoo へ").font(.largeTitle.bold())
                        Text("いつもの会話を、iPhone で。")
                        if let error = session.error {
                            Text(error).font(.callout).foregroundStyle(.red).accessibilityIdentifier("login-error")
                        }
                        if let prompt = session.deviceAuthorization {
                            Text("確認コード").font(.headline)
                            Text(prompt.userCode)
                                .font(.largeTitle.monospaced().bold())
                                .textSelection(.enabled)
                                .accessibilityIdentifier("device-user-code")
                            Text("認証画面のコードと一致することを確認して、ログインを承認してください。")
                            Text(prompt.verificationURI.absoluteString)
                                .font(.footnote).textSelection(.enabled)
                            Button("認証画面を開く") { showingVerification = true }
                                .buttonStyle(.borderedProminent).buttonBorderShape(.capsule)
                                .controlSize(.large)
                                .accessibilityIdentifier("device-verify")
                        }
                        if session.connecting {
                            ProgressView(session.deviceAuthorization == nil ? "接続しています…" : "承認を待っています…")
                            Button("ログインを中止", systemImage: "xmark") { session.cancelLogin() }
                                .labelStyle(.iconOnly).buttonStyle(.bordered).buttonBorderShape(.circle)
                                .accessibilityIdentifier("cancel-login")
                        } else {
                            Button("ログイン") { session.startLogin() }
                                .buttonStyle(.borderedProminent).buttonBorderShape(.capsule)
                                .controlSize(.large)
                                .accessibilityIdentifier("login")
                        }
                    }
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
                    .padding(24)
                }
            }
        }
        .sheet(isPresented: $showingVerification) {
            if let prompt = session.deviceAuthorization {
                VerificationBrowser(url: prompt.verificationURL)
                    .ignoresSafeArea()
            }
        }
        .onChange(of: session.deviceAuthorization) { _, prompt in
            if prompt == nil { showingVerification = false }
        }
    }
}

private struct VerificationBrowser: UIViewControllerRepresentable {
    let url: URL
    @Environment(\.dismiss) private var dismiss

    func makeCoordinator() -> Coordinator { Coordinator { dismiss() } }

    func makeUIViewController(context: Context) -> SFSafariViewController {
        let controller = SFSafariViewController(url: url)
        controller.delegate = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: SFSafariViewController, context: Context) { }

    final class Coordinator: NSObject, SFSafariViewControllerDelegate {
        private let finish: () -> Void
        init(finish: @escaping () -> Void) { self.finish = finish }
        func safariViewControllerDidFinish(_ controller: SFSafariViewController) { finish() }
    }
}
