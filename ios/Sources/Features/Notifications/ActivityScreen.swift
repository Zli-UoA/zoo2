import SwiftUI

struct ActivityScreen: View {
    @Environment(ChannelViewModel.self) private var channels
    @Environment(NotificationsViewModel.self) private var model
    @Environment(SessionController.self) private var session
    @State private var path: [ChannelRoute] = []

    var body: some View {
        @Bindable var model = model
        NavigationStack(path: $path) {
            List {
                Toggle("未読のみ", isOn: $model.onlyUnread)
                let visible = model.activities
                if visible.isEmpty {
                    ContentUnavailableView("通知はありません", systemImage: "bell.badge", description: Text("メンションや返信がここに届きます。"))
                }
                ForEach(visible) { activity in
                    Button {
                        guard let channel = channels.channel(activity.channel_id) else {
                            session.error = "この通知のチャンネルはまだ読み込まれていないか、閲覧できません。"; return
                        }
                        path.append(ChannelRoute(channel: channel, messageID: activity.message_id))
                        Task { _ = await session.perform { try await model.read(activity) } }
                    } label: {
                        HStack(alignment: .top, spacing: 12) {
                            Avatar(name: channels.name(activity.actor_id))
                            VStack(alignment: .leading, spacing: 5) {
                                Text(channels.name(activity.actor_id)).font(.subheadline.bold())
                                Text(activity.summary).font(.subheadline)
                                if let channel = channels.channel(activity.channel_id) {
                                    Text(channels.title(channel)).font(.caption).foregroundStyle(.secondary)
                                }
                                if let date = ZooDate.parse(activity.created_at) { Text(date, style: .relative).font(.caption2).foregroundStyle(.secondary) }
                            }
                            Spacer()
                            if !activity.is_read { Circle().fill(Color.zooAccent).frame(width: 8, height: 8).accessibilityLabel("未読") }
                        }.padding(.vertical, 6).foregroundStyle(.primary)
                    }
                }
            }
            .navigationTitle("通知")
            .navigationDestination(for: ChannelRoute.self) { ChannelScreen(route: $0) }
            .refreshable { session.reconnect() }
        }
    }
}
