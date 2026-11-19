import SwiftUI
import SafariServices

@main
struct ZooApp: App {
    @UIApplicationDelegateAdaptor(NotificationDelegate.self) private var delegate
    private var session: AppSession { delegate.session }
    @Environment(\.scenePhase) private var scenePhase
    @AppStorage(AppearancePreference.key) private var appearance = AppearancePreference.system

    var body: some Scene {
        WindowGroup {
            RootView(session: session)
                .preferredColorScheme(appearance.colorScheme)
                .environment(\.locale, Locale(identifier: "ja_JP"))
                .tint(AppDesign.navigationTint)
                .task {
                    guard !AppSession.isUITesting else { return }
                    delegate.systemCalls.setForeground(scenePhase == .active)
                    await session.resume()
                }
                .onChange(of: scenePhase) { _, value in
                    if !AppSession.isUITesting { delegate.systemCalls.setForeground(value == .active) }
                    if value == .active, !AppSession.isUITesting { Task { await session.resume() } }
                }
                .onChange(of: session.notifications.destination) { _, _ in
                    Task { await session.deliverDestination() }
                }
        }
    }
}

@MainActor
enum AppOrientation {
    private static var attachmentScenes: [UUID: UIWindowScene] = [:]

    static func supportedOrientations(in scene: UIWindowScene?) -> UIInterfaceOrientationMask {
        guard UIDevice.current.userInterfaceIdiom == .phone else { return .allButUpsideDown }
        return attachmentScenes.values.contains { $0 === scene } ? .allButUpsideDown : .portrait
    }

    static func beginAttachment(_ id: UUID, in scene: UIWindowScene?) {
        guard let scene, attachmentScenes[id] == nil else { return }
        attachmentScenes[id] = scene
        update(scene)
    }

    static func endAttachment(_ id: UUID) {
        guard let scene = attachmentScenes.removeValue(forKey: id) else { return }
        update(scene)
    }

    private static func update(_ scene: UIWindowScene) {
        var controller = scene.keyWindow?.rootViewController
        while let current = controller {
            current.setNeedsUpdateOfSupportedInterfaceOrientations()
            controller = current.presentedViewController
        }
        // 閲覧終了後は端末を持ち直さなくても通常画面を縦向きへ戻す。
        scene.requestGeometryUpdate(.iOS(interfaceOrientations: supportedOrientations(in: scene)))
    }
}

struct AttachmentOrientation: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> Controller { Controller() }
    func updateUIViewController(_ controller: Controller, context: Context) {}
    static func dismantleUIViewController(_ controller: Controller, coordinator: ()) {
        AppOrientation.endAttachment(controller.id)
    }

    final class Controller: UIViewController {
        let id = UUID()

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            AppOrientation.beginAttachment(id, in: view.window?.windowScene)
        }

        override func viewDidDisappear(_ animated: Bool) {
            super.viewDidDisappear(animated)
            AppOrientation.endAttachment(id)
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
                ViewThatFits(in: .vertical) {
                    VStack(spacing: 24) {
                        welcomeHeader
                        if let prompt = session.deviceAuthorization {
                            authorizationDetails(prompt)
                        }
                        loginActions
                    }
                    HStack(spacing: 24) {
                        if let prompt = session.deviceAuthorization {
                            authorizationDetails(prompt)
                        } else {
                            welcomeHeader
                        }
                        loginActions
                    }
                }
                .multilineTextAlignment(.center)
                .padding(24)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
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

    private var welcomeHeader: some View {
        VStack(spacing: 24) {
            Image(systemName: "pawprint.fill").font(.system(size: 64)).foregroundStyle(.tint)
            Text("ようこそ Zoo へ").font(.largeTitle.bold())
            Text("いつもの会話を、iPhone で。")
        }
    }

    private func authorizationDetails(_ prompt: DeviceAuthorizationPrompt) -> some View {
        VStack(spacing: 12) {
            Text("確認コード").font(.headline)
            Text(prompt.userCode)
                .font(.largeTitle.monospaced().bold())
                .textSelection(.enabled)
                .accessibilityIdentifier("device-user-code")
            Text("認証画面のコードと一致することを確認して、ログインを承認してください。")
            Text(prompt.verificationURI.absoluteString)
                .font(.footnote).textSelection(.enabled)
        }
    }

    private var loginActions: some View {
        VStack(spacing: 24) {
            if let error = session.error {
                Text(error).font(.callout).foregroundStyle(.red).accessibilityIdentifier("login-error")
            }
            if session.deviceAuthorization != nil {
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
