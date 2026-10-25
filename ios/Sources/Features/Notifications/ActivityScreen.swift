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
                let visible = model.activities
                if visible.isEmpty {
                    ContentUnavailableView(model.onlyUnread ? "未読の通知はありません" : "通知はありません", systemImage: model.onlyUnread ? "checkmark.circle" : "bell", description: Text(model.onlyUnread ? "すべて確認済みです。" : "メンションや返信がここに届きます。"))
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
                                Text(activity.summary).font(.body)
                                if let channel = channels.channel(activity.channel_id) {
                                    Label(channels.title(channel), systemImage: channel.is_dm ? "bubble.left" : (channel.is_private ? "lock" : "number"))
                                        .font(.caption).foregroundStyle(.secondary)
                                }
                                if let date = ZooDate.parse(activity.created_at) { Text(date.formatted(.dateTime.month().day().hour().minute().locale(Locale(identifier: "ja_JP")))).font(.caption).foregroundStyle(.secondary) }
                            }
                            Spacer()
                            if !activity.is_read { Circle().fill(Color.zooAccent).frame(width: 8, height: 8).accessibilityLabel("未読") }
                        }.padding(.vertical, 10).foregroundStyle(.primary)
                            .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                    }
                    .listRowBackground(activity.is_read ? Color.zooBackground : Color.zooBrandMuted.opacity(0.3))
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(Color.zooBackground)
            .safeAreaInset(edge: .top, spacing: 0) {
                VStack(spacing: 0) {
                    Picker("通知の表示", selection: $model.onlyUnread) {
                        Text("すべて").tag(false)
                        Text("未読").tag(true)
                    }
                    .pickerStyle(.segmented).padding(.horizontal, 16).padding(.vertical, 10)
                    .accessibilityIdentifier("notificationFilter")
                    Divider()
                }.background(Color.zooBackground)
            }
            .navigationTitle("通知")
            .navigationBarTitleDisplayMode(.inline)
            .navigationDestination(for: ChannelRoute.self) { ChannelScreen(route: $0) }
            .refreshable { session.reconnect() }
        }
    }
}
