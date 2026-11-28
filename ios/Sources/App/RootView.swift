import SwiftUI

struct RootView: View {
    @Bindable var session: AppSession

    var body: some View {
        Group {
            if let workspace = session.workspace {
                WorkspaceView(workspace: workspace, session: session)
                    .environment(\.remoteImages, workspace.images)
            } else if session.isStarting || session.restoring {
                LaunchScreenView()
            } else {
                GeometryReader { geometry in
                    ScrollView {
                        VStack(spacing: 24) {
                            welcomeHeader
                            loginActions
                        }
                        .frame(maxWidth: .infinity)
                        .padding(24)
                        .frame(minHeight: geometry.size.height)
                    }
                    .accessibilityIdentifier("login-screen")
                }
                .multilineTextAlignment(.center)
            }
        }
        #if DEBUG
        .overlay(alignment: .topTrailing) {
            if AppSession.isUITesting {
                if ProcessInfo.processInfo.arguments.contains("--uitesting-paged-history") {
                    Button("取得を再開") { PreviewTestControl.releaseHistory() }
                        .disabled(!PreviewTestControl.shared.waitingForHistory)
                        .accessibilityIdentifier("preview-release-history")
                }
                if ProcessInfo.processInfo.arguments.contains("--uitesting-long-history"),
                   ProcessInfo.processInfo.arguments.contains("--uitesting-live-message") {
                    Button(PreviewTestControl.shared.incomingDelivered ? "配信済み" : "新着を配信") {
                        PreviewTestControl.shared.incomingReleased = true
                    }
                    .accessibilityIdentifier("preview-release-message")
                }
                if ProcessInfo.processInfo.arguments.contains("--uitesting-typing-transition") {
                    Button("入力中表示を終了") { PreviewTestControl.typingVisible = false }
                        .accessibilityIdentifier("preview-end-typing")
                }
                if ProcessInfo.processInfo.arguments.contains("--uitesting-orientation") {
                    InterfaceOrientationProbe().frame(width: 1, height: 1)
                }
            }
        }
        #endif
        .background(AppDesign.background.ignoresSafeArea())
    }

    private var welcomeHeader: some View {
        VStack(spacing: 24) {
            Image(systemName: "pawprint.fill").font(.system(size: 64)).foregroundStyle(.tint)
            Text("ようこそ Zoo へ").font(.largeTitle.bold())
            Text("いつもの会話を、手元で。")
        }
    }

    private var loginActions: some View {
        VStack(spacing: 24) {
            if let error = session.error {
                Text(error).font(.callout).foregroundStyle(.red).accessibilityIdentifier("login-error")
            }
            if session.connecting {
                ProgressView("認証画面でログインしてください…")
                Button("ログインを中止", systemImage: "xmark") { session.cancelLogin() }
                    .labelStyle(.iconOnly).buttonStyle(.bordered).buttonBorderShape(.circle)
                    .accessibilityIdentifier("cancel-login")
            } else {
                Button("パスキーでログイン", systemImage: "person.badge.key.fill") { session.startLogin() }
                    .buttonStyle(.borderedProminent).buttonBorderShape(.capsule)
                    .tint(AppDesign.primary).foregroundStyle(AppDesign.primaryForeground)
                    .controlSize(.large)
                    .accessibilityIdentifier("login")
            }
        }
    }
}

#if DEBUG
private struct InterfaceOrientationProbe: UIViewRepresentable {
    func makeUIView(context: Context) -> Probe { Probe() }
    func updateUIView(_ view: Probe, context: Context) { view.setNeedsLayout() }

    final class Probe: UIView {
        override init(frame: CGRect) {
            super.init(frame: frame)
            isAccessibilityElement = true
            accessibilityIdentifier = "interface-orientation"
        }
        required init?(coder: NSCoder) { nil }
        override var accessibilityLabel: String? {
            get {
                // 同じ寸法の回転では再配置されないため、照会時の向きを返す。
                switch window?.windowScene?.effectiveGeometry.interfaceOrientation {
                case .portraitUpsideDown: "portraitUpsideDown"
                case .portrait: "portrait"
                case .landscapeLeft: "landscapeLeft"
                case .landscapeRight: "landscapeRight"
                default: "unknown"
                }
            }
            set { super.accessibilityLabel = newValue }
        }
    }
}
#endif
