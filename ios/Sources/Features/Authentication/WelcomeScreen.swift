import SwiftUI

struct WelcomeScreen: View {
    @Environment(SessionController.self) private var session
    @State private var showSettings = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    ZooMark(size: 64).padding(.top, 40)
                    VStack(alignment: .leading, spacing: 12) {
                        Text("ようこそ Zoo へ").font(.largeTitle.weight(.bold))
                        Text("Zli の会話を、いつでも手元に。\nチャンネル、DM、スレッドでつながろう。")
                            .font(.subheadline).foregroundStyle(.secondary)
                    }
                    VStack(spacing: 14) {
                        Button {
                            if session.settings.clientID.isEmpty { showSettings = true }
                            else { Task { await session.connect(interactive: true) } }
                        } label: {
                            HStack {
                                if session.isConnecting { ProgressView().tint(.zooBrandForeground) }
                                Text("Zli アカウントでログイン").fontWeight(.semibold)
                                Image(systemName: "arrow.right")
                            }.frame(maxWidth: .infinity).padding(.vertical, 10)
                        }.foregroundStyle(Color.zooBrandForeground).buttonStyle(.borderedProminent).controlSize(.large).disabled(session.isConnecting)
                        Button("デモを試す") { session.loadDemo() }
                            .frame(maxWidth: .infinity, minHeight: 44)
                            .buttonStyle(.bordered).accessibilityIdentifier("demoButton")
                            .disabled(session.isConnecting)
                        Text("デモはサーバーに接続しません。")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(28).frame(maxWidth: 540)
            }
            .frame(maxWidth: .infinity)
            .background(Color.zooBackground)
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("接続設定", systemImage: "gearshape") { showSettings = true } } }
            .sheet(isPresented: $showSettings) { ConnectionScreen(settings: session.settings) }
        }
    }
}
