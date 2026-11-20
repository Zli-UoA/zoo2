import SwiftUI

struct TabletWorkspaceView: View {
    @Bindable var workspace: WorkspaceStore
    let session: AppSession
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        HStack(spacing: 0) {
            destinations
                // 開閉中のサイドバーをタブ列の背後へ隠す。
                .zIndex(1)
            Divider()
            switch workspace.selectedTab {
            case .channels:
                TabletConversations(workspace: workspace, directMessages: false)
            case .directMessages:
                TabletConversations(workspace: workspace, directMessages: true)
            case .notifications:
                NavigationStack {
                    InboxView(store: workspace.inbox, workspace: workspace, isActive: scenePhase == .active,
                              openChat: workspace.openInboxMessage)
                        .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
                }
            case .profile:
                NavigationStack(path: $workspace.profilePath) {
                    ProfileView(workspace: workspace, session: session)
                        .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
                }
            }
        }
    }

    private var destinations: some View {
        VStack(spacing: 12) {
            Image(systemName: "pawprint.fill").font(.title2).foregroundStyle(.tint)
                .frame(height: 44).accessibilityHidden(true)
            destination("チャンネル", symbol: "number", tab: .channels)
            destination("DM", symbol: "bubble.left.and.bubble.right.fill", tab: .directMessages)
            destination("通知", symbol: "bell.fill", tab: .notifications)
            Spacer(minLength: 16)
            destination("プロフィール", symbol: "person.crop.circle.fill", tab: .profile)
        }
        .padding(.horizontal, 8).padding(.vertical, 12)
        .frame(width: 88)
        .background(Color(.secondarySystemBackground))
    }

    private func destination(_ title: String, symbol: String, tab: WorkspaceTab) -> some View {
        Button {
            workspace.selectedTab = tab
        } label: {
            VStack(spacing: 6) {
                Image(systemName: symbol).font(.title3)
                    .overlay(alignment: .topTrailing) {
                        if tab == .notifications, workspace.inbox.unreadCount > 0 {
                            Text("\(min(workspace.inbox.unreadCount, 99))")
                                .font(.caption2.bold()).foregroundStyle(.white)
                                .padding(3).background(.red, in: Circle()).offset(x: 10, y: -8)
                        }
                    }
                Text(title).font(.caption2).lineLimit(2).minimumScaleFactor(0.8)
            }
            .frame(maxWidth: .infinity, minHeight: 58)
            .foregroundStyle(workspace.selectedTab == tab ? Color.accentColor : .secondary)
            .background(workspace.selectedTab == tab ? Color.accentColor.opacity(0.12) : .clear,
                        in: RoundedRectangle(cornerRadius: 12))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityAddTraits(workspace.selectedTab == tab ? .isSelected : [])
        .accessibilityIdentifier("ipad-tab-\(tab)")
    }
}

private struct TabletConversations: View {
    @Bindable var workspace: WorkspaceStore
    let directMessages: Bool
    @State private var visibility = NavigationSplitViewVisibility.all

    private var routes: [ChatRoute] { directMessages ? workspace.dmPath : workspace.channelPath }

    private var detailPath: Binding<[ChatRoute]> {
        Binding(get: { Array(routes.dropFirst()) }, set: { path in
            guard let root = routes.first else { return }
            if directMessages { workspace.dmPath = [root] + path }
            else { workspace.channelPath = [root] + path }
        })
    }

    var body: some View {
        NavigationSplitView(columnVisibility: $visibility) {
            ConversationDirectory(workspace: workspace, directMessages: directMessages, isSidebar: true)
                .navigationSplitViewColumnWidth(min: 220, ideal: 270, max: 340)
        } detail: {
            NavigationStack(path: detailPath) {
                Group {
                    if let route = routes.first {
                        ConversationDestination(workspace: workspace, route: route)
                    } else {
                        ContentUnavailableView(directMessages ? "DMを選択" : "チャンネルを選択",
                                               systemImage: directMessages ? "bubble.left.and.bubble.right" : "number",
                                               description: Text("左の一覧から会話を開けます。"))
                            .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
                    }
                }
                .navigationDestination(for: ChatRoute.self) { route in
                    ConversationDestination(workspace: workspace, route: route)
                }
            }
            // 会話を切り替えた際に、前の会話のスクロール位置や返信欄を引き継がない。
            .id(routes.first?.channelID)
        }
        .navigationSplitViewStyle(.balanced)
    }
}
