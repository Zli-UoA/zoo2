import SwiftUI

struct ChannelSettingsScreen: View {
    @Environment(ChannelViewModel.self) private var model
    @Environment(SessionController.self) private var session
    @Environment(\.dismiss) private var dismiss
    let channelID: String
    @State private var name = ""
    @State private var description = ""
    @State private var busy = false
    @State private var confirmingLeave = false
    @State private var confirmingArchive = false

    var body: some View {
        NavigationStack {
            Form {
                if let channel = model.channel(channelID) {
                    if model.canManage(channel) {
                        Section("チャンネル設定") {
                            TextField("名前", text: $name)
                            TextField("説明", text: $description, axis: .vertical).lineLimit(2...4)
                            Button("変更を保存") {
                                run { try await model.updateChannel(channel, name: name.trimmingCharacters(in: .whitespacesAndNewlines), description: description) }
                            }.disabled(busy || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || name.unicodeScalars.count > 80 || description.unicodeScalars.count > 250)
                        }
                    }
                    Section("メンバー") {
                        ForEach(model.memberships.filter { $0.channel_id == channelID }, id: \.user_id) { membership in
                            HStack {
                                Avatar(name: model.name(membership.user_id))
                                Text(model.name(membership.user_id))
                                if membership.user_id == model.me?.id { Text("あなた").font(.caption).foregroundStyle(.secondary) }
                            }
                        }
                        if model.canInvite(channel), channel.archive_at == nil {
                            NavigationLink { InviteScreen(channel: channel) } label: {
                                Label("メンバーを追加", systemImage: "person.badge.plus")
                            }
                        }
                    }
                    Section {
                        if model.canManage(channel) {
                            Button(channel.archive_at == nil ? "アーカイブする" : "アーカイブを解除") { confirmingArchive = true }.disabled(busy)
                        }
                        if model.joined(channelID) {
                            Button("チャンネルから退出", role: .destructive) { confirmingLeave = true }.disabled(busy)
                        }
                    }
                    .confirmationDialog("チャンネルから退出しますか？", isPresented: $confirmingLeave, titleVisibility: .visible) {
                        Button("退出", role: .destructive) { run { try await model.leave(channel); dismiss() } }
                    } message: { Text("プライベートチャンネルへの再参加には、メンバーからの招待が必要です。") }
                    .confirmationDialog(channel.archive_at == nil ? "アーカイブしますか？" : "アーカイブを解除しますか？", isPresented: $confirmingArchive, titleVisibility: .visible) {
                        Button("変更する") { run { try await model.setArchived(channel, archived: channel.archive_at == nil) } }
                    }
                } else {
                    ContentUnavailableView("チャンネルを表示できません", systemImage: "lock")
                }
            }
            .navigationTitle("チャンネル情報").navigationBarTitleDisplayMode(.inline)
            .onAppear {
                name = model.channel(channelID)?.channel_name ?? ""
                description = model.channel(channelID)?.description ?? ""
            }
            .interactiveDismissDisabled(busy)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("閉じる") { dismiss() }.disabled(busy) } }
        }
    }

    private func run(_ operation: @escaping @MainActor () async throws -> Void) {
        busy = true
        Task { _ = await session.perform(operation); busy = false }
    }
}

struct InviteScreen: View {
    @Environment(ChannelViewModel.self) private var model
    @Environment(SessionController.self) private var session
    let channel: Channel
    @State private var search = ""
    @State private var adding: String?
    var body: some View {
        List(model.people.filter { person in
            !model.memberships.contains { $0.channel_id == channel.id && $0.user_id == person.id } &&
                (search.isEmpty || person.display_name.localizedCaseInsensitiveContains(search))
        }) { person in
            Button {
                adding = person.id
                Task { _ = await session.perform { try await model.invite(person, to: channel) }; adding = nil }
            } label: {
                HStack {
                    Avatar(name: person.display_name)
                    Text(person.display_name).foregroundStyle(.primary)
                    Spacer()
                    if adding == person.id { ProgressView() } else { Image(systemName: "plus.circle") }
                }
            }.disabled(adding != nil)
        }
        .searchable(text: $search, prompt: "メンバーを検索")
        .navigationTitle("メンバーを追加")
    }
}
