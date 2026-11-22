import SwiftUI
import SafariServices

struct RootView: View {
    @Bindable var session: AppSession
    @State private var showingVerification = false

    var body: some View {
        Group {
            if let workspace = session.workspace {
                WorkspaceView(workspace: workspace, session: session)
                    .environment(\.remoteImages, workspace.images)
            } else if session.isStarting || session.restoring {
                LaunchScreenView()
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
        .background(AppDesign.background.ignoresSafeArea())
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
            Text("いつもの会話を、手元で。")
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
                    .tint(AppDesign.primary).foregroundStyle(AppDesign.primaryForeground)
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
                    .tint(AppDesign.primary).foregroundStyle(AppDesign.primaryForeground)
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
