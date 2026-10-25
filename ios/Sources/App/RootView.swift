import SwiftUI

struct RootView: View {
    @Environment(SessionController.self) private var session

    var body: some View {
        @Bindable var session = session
        Group {
            if let context = session.context {
                VStack(spacing: 0) {
                    VStack(spacing: 0) {
                        if session.isDemo {
                            Text("デモ · この端末だけのサンプルデータ")
                                .font(.caption).frame(maxWidth: .infinity).padding(6)
                                .background(Color.orange.opacity(0.12))
                        }
                        if context.hasSyncErrors {
                            HStack {
                                Image(systemName: "wifi.exclamationmark")
                                Text("同期が停止しています").font(.caption)
                                Spacer()
                                Button("再接続") { session.reconnect() }.font(.caption.bold())
                            }.padding(10).background(Color.orange.opacity(0.15))
                            .accessibilityElement(children: .contain)
                        }
                    }
                    TabView {
                        ChannelListScreen(directMessages: false)
                            .tabItem { Label("ホーム", systemImage: "square.grid.2x2") }
                        ChannelListScreen(directMessages: true)
                            .tabItem { Label("DM", systemImage: "bubble.left.and.bubble.right") }
                        ActivityScreen()
                            .tabItem { Label("通知", systemImage: "bell") }.badge(context.notifications.pendingCount)
                        ProfileScreen()
                            .tabItem { Label("あなた", systemImage: "person.crop.circle") }
                    }
                    .environment(context)
                    .environment(context.channels)
                    .environment(context.notifications)
                    .environment(context.profile)
                }
            } else { WelcomeScreen() }
        }
        .alert("確認してください", isPresented: Binding(get: { session.error != nil }, set: { if !$0 { session.error = nil } })) {
            Button("閉じる", role: .cancel) { session.error = nil }
        } message: { Text(session.error ?? "") }
    }
}
