import SwiftUI

struct RootView: View {
    @Environment(SessionController.self) private var session

    @State private var selection = 0

    var body: some View {
        @Bindable var session = session
        Group {
            if let context = session.context {
                VStack(spacing: 0) {
                    VStack(spacing: 0) {
                        if session.isDemo {
                            Label("デモ · この端末だけのサンプルデータ", systemImage: "play.circle")
                                .font(.caption).dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                                .frame(maxWidth: .infinity).padding(6)
                                .foregroundStyle(.secondary)
                                .background(Color.zooMuted)
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
                    TabView(selection: $selection) {
                        ChannelListScreen(directMessages: false)
                            .tabItem { Label("チャンネル", systemImage: "number") }.tag(0)
                        ChannelListScreen(directMessages: true)
                            .tabItem { Label("DM", systemImage: "bubble.left.and.bubble.right") }.tag(1)
                        ActivityScreen()
                            .tabItem { Label("通知", systemImage: "bell") }.tag(2)
                            .badge(context.notifications.pendingCount)
                        ProfileScreen()
                            .tabItem { Label("あなた", systemImage: "person.crop.circle") }.tag(3)
                    }
                    .environment(context)
                    .environment(context.channels)
                    .environment(context.notifications)
                    .environment(context.profile)
                    .sheet(isPresented: Binding(get: {
                        guard let target = session.notifications.destination,
                              target.userID == context.profile.me?.id else { return false }
                        return context.channels.channel(target.channelID) != nil
                    }, set: { if !$0 { session.notifications.destination = nil } })) {
                        if let target = session.notifications.destination,
                           target.userID == context.profile.me?.id,
                           let channel = context.channels.channel(target.channelID) {
                            NavigationStack {
                                ChannelScreen(route: ChannelRoute(channel: channel, messageID: target.messageID))
                                    .toolbar { ToolbarItem(placement: .cancellationAction) {
                                        Button("閉じる") { session.notifications.destination = nil }
                                    } }
                            }.environment(context).environment(context.channels)
                        }
                    }
                }
            } else { WelcomeScreen() }
        }
        .alert("確認してください", isPresented: Binding(get: { session.error != nil }, set: { if !$0 { session.error = nil } })) {
            Button("閉じる", role: .cancel) { session.error = nil }
        } message: { Text(session.error ?? "") }
    }

}
