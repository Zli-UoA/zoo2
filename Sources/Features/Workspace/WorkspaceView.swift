import AVFAudio
import SwiftUI

struct WorkspaceView: View {
    @Bindable var workspace: WorkspaceStore
    let session: AppSession
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    var body: some View {
        Group {
            if UIDevice.current.userInterfaceIdiom == .pad, horizontalSizeClass == .regular {
                TabletWorkspaceView(workspace: workspace, session: session)
            } else {
                compactTabs
            }
        }
        .sheet(isPresented: Binding(get: { workspace.calls.showingCall }, set: { workspace.calls.showingCall = $0 })) {
            CallView(store: workspace.calls, workspace: workspace)
        }
        .onReceive(NotificationCenter.default.publisher(for: AVAudioSession.interruptionNotification)) { notification in
            if let type = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
               type == AVAudioSession.InterruptionType.began.rawValue,
               workspace.systemCalls?.isManagingCall != true { workspace.calls.finish() }
        }
        .task(id: scenePhase) {
            if scenePhase == .active, workspace.systemCalls == nil { await workspace.calls.runInvitations() }
        }
        .modifier(ConversationSync(workspace: workspace))
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.calls.runActiveCalls() }
        }
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.run() }
        }
        .task(id: scenePhase) {
            if scenePhase == .active { await workspace.inbox.run() }
        }
    }

    private var compactTabs: some View {
        TabView(selection: $workspace.selectedTab) {
            Tab("チャンネル", systemImage: "number", value: WorkspaceTab.channels) {
                chatStack(path: $workspace.channelPath, directMessages: false)
            }
            Tab("DM", systemImage: "bubble.left.and.bubble.right.fill", value: WorkspaceTab.directMessages) {
                chatStack(path: $workspace.dmPath, directMessages: true)
            }
            Tab("通知", systemImage: "bell.fill", value: WorkspaceTab.notifications) {
                NavigationStack {
                    InboxView(store: workspace.inbox, workspace: workspace,
                              isActive: workspace.selectedTab == .notifications && scenePhase == .active,
                              openChat: workspace.openInboxMessage)
                        .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
                }
            }
            .badge(workspace.inbox.unreadCount)
            Tab("プロフィール", systemImage: "person.crop.circle.fill", value: WorkspaceTab.profile) {
                NavigationStack {
                    ProfileView(workspace: workspace, session: session)
                        .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
                }
            }
        }
    }

    private func chatStack(path: Binding<[ChatRoute]>, directMessages: Bool) -> some View {
        NavigationStack(path: path) {
            ConversationDirectory(workspace: workspace, directMessages: directMessages)
                .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
                .navigationDestination(for: ChatRoute.self) { route in
                    ConversationDestination(workspace: workspace, route: route)
                }
        }
    }
}
