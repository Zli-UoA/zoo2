import SwiftUI

struct WelcomeScreen: View {
    @Environment(SessionController.self) private var session
    @State private var showSettings = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 28) {
                    ZooMark(size: 84).padding(.top, 55)
                    VStack(alignment: .leading, spacing: 12) {
                        Text("つくる。話す。\nここに集まる。").font(.system(.largeTitle, design: .rounded, weight: .bold))
                        Text("Zli のためのコミュニケーション。\n日々の雑談から、次のアイデアまで。")
                            .font(.body).foregroundStyle(.secondary).lineSpacing(5)
                    }
                    VStack(alignment: .leading, spacing: 20) {
                        Label("話題ごとのチャンネル", systemImage: "number")
                        Label("気軽につながる DM", systemImage: "bubble.left.and.bubble.right")
                        Label("会話をつなぐスレッド", systemImage: "arrowshape.turn.up.left")
                    }.font(.subheadline).padding(22).frame(maxWidth: .infinity, alignment: .leading)
                        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 24))
                    VStack(spacing: 14) {
                        Button {
                            if session.settings.clientID.isEmpty { showSettings = true }
                            else { Task { await session.connect(interactive: true) } }
                        } label: {
                            HStack {
                                if session.isConnecting { ProgressView().tint(.white) }
                                Text("Zli アカウントでログイン").fontWeight(.semibold)
                                Image(systemName: "arrow.right")
                            }.frame(maxWidth: .infinity).padding(.vertical, 10)
                        }.buttonStyle(.borderedProminent).controlSize(.large).disabled(session.isConnecting)
                        Button("デモを試す") { session.loadDemo() }.accessibilityIdentifier("demoButton")
                            .disabled(session.isConnecting)
                        Text("デモはサーバーに接続しません。")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(28).frame(maxWidth: 540)
            }
            .frame(maxWidth: .infinity)
            .background(Color(uiColor: .systemGroupedBackground))
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("接続設定", systemImage: "gearshape") { showSettings = true } } }
            .sheet(isPresented: $showSettings) { ConnectionScreen(settings: session.settings) }
        }
    }
}
