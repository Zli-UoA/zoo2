import SwiftUI

struct ChannelScreen: View {
    @Environment(WorkspaceContext.self) private var context
    let route: ChannelRoute
    @State private var model: ConversationViewModel?

    var body: some View {
        Group {
            if let model {
                ChannelContent(route: route).environment(model)
            } else { ProgressView() }
        }
        .task(id: route.channel.id) { model = context.conversation(route.channel.id) }
    }
}

private struct ChannelContent: View {
    @Environment(ChannelViewModel.self) private var channels
    @Environment(ConversationViewModel.self) private var conversation
    @Environment(SessionController.self) private var session
    let route: ChannelRoute
    @State private var thread: Message?
    @State private var showMembers = false
    @State private var reader = UUID()
    @State private var consumedTarget = false
    @State private var busy = false
    private var channel: Channel { channels.channel(route.channel.id) ?? route.channel }
    private var canPost: Bool { channels.channel(channel.id) != nil && channels.joined(channel.id) && channel.archive_at == nil && !conversation.accessDenied }

    var body: some View {
        VStack(spacing: 0) {
            if let description = channel.description, !description.isEmpty {
                Text(description).font(.caption).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18).padding(.vertical, 8)
                Divider()
            }
            if conversation.accessDenied {
                ContentUnavailableView("チャンネルを表示できません", systemImage: "lock", description: Text("参加状態を確認するか、再接続してください。"))
            } else {
                MessageTimeline(channelID: channel.id, parentID: nil, targetID: route.messageID, canPost: canPost) { thread = $0 }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if canPost { Composer(channelID: channel.id, parentID: nil) }
            else if channel.archive_at != nil {
                Label("アーカイブ済みのチャンネルです", systemImage: "archivebox").font(.callout).padding()
            } else if !channel.is_private && !channel.is_dm && channels.channel(channel.id) != nil {
                Button("チャンネルに参加") {
                    busy = true
                    Task { _ = await session.perform { try await channels.join(channel) }; busy = false }
                }.buttonStyle(.borderedProminent).padding().disabled(busy)
            }
        }
        .navigationTitle(channels.title(channel)).navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) { Button("メンバー", systemImage: "person.2") { showMembers = true } }
        }
        .onAppear { conversation.open(reader: reader) }
        .onDisappear { conversation.close(reader: reader) }
        .onChange(of: conversation.timeline, initial: true) { _, rows in
            guard !consumedTarget, let target = route.messageID,
                  let message = rows.first(where: { $0.id == target }) else { return }
            if let parent = message.parent_id { thread = rows.first { $0.id == parent } }
            consumedTarget = true
        }
        .sheet(item: $thread) { root in
            NavigationStack {
                VStack(spacing: 0) {
                    MessageTimeline(channelID: channel.id, parentID: root.id, targetID: route.messageID, canPost: canPost) { _ in }
                }
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    if canPost && conversation.timeline.first(where: { $0.id == root.id })?.deleted_at == nil {
                        Composer(channelID: channel.id, parentID: root.id)
                    }
                }
                .navigationTitle("スレッド").navigationBarTitleDisplayMode(.inline)
                .toolbar { ToolbarItem(placement: .confirmationAction) { Button("閉じる") { thread = nil } } }
            }.presentationDragIndicator(.visible)
        }
        .sheet(isPresented: $showMembers) {
            ChannelSettingsScreen(channelID: channel.id)
        }
    }
}
