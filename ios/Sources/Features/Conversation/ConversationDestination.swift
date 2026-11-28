import SwiftUI

struct ConversationDestination: View {
    let workspace: WorkspaceStore
    let route: ChatRoute

    var body: some View {
        Group {
            if let channel = workspace.channels.first(where: { $0.id == route.channelID }) {
                ConversationView(store: workspace.conversation(for: channel), workspace: workspace, route: route)
            } else {
                ContentUnavailableView("会話を表示できません", systemImage: "bubble.left.and.exclamationmark.bubble.right")
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) { CallBanner(workspace: workspace) }
    }
}
