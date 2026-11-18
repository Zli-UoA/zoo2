import SwiftUI

struct ChannelDetailsView: View {
    let channel: Channel
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section("会話について") {
                    Label(workspace.name(for: channel), systemImage: channel.symbol).font(.headline)
                    if let description = channel.description, !description.isEmpty {
                        Text(description).textSelection(.enabled)
                    }
                    LabeledContent("種類", value: channel.is_dm ? "ダイレクトメッセージ" : channel.is_private ? "非公開チャンネル" : "公開チャンネル")
                    if channel.archive_at != nil { Label("アーカイブ済み", systemImage: "archivebox") }
                    if channel.encrypted == true { Label("暗号化された会話", systemImage: "lock.shield") }
                    if workspace.pinnedChannelIDs.contains(channel.id) { Label("ピン留め済み", systemImage: "pin.fill") }
                }
                Section("メンバー") {
                    if let error = workspace.memberErrors[channel.id] {
                        Text(error).foregroundStyle(.secondary)
                        Button("再読み込み") { Task { await workspace.loadMembers(channelID: channel.id) } }
                    } else if let members = workspace.channelMembers[channel.id] {
                        Text("\(members.count)人").foregroundStyle(.secondary)
                        ForEach(members) { PersonRow(person: $0) }
                    } else { ProgressView("メンバーを読み込み中…") }
                }
            }
            .navigationTitle("チャンネル詳細").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("閉じる") { dismiss() } } }
            .task { await workspace.loadMembers(channelID: channel.id) }
        }
    }
}

struct ChannelPinButton: View {
    let channel: Channel
    let workspace: WorkspaceStore

    var body: some View {
        let pinned = workspace.pinnedChannelIDs.contains(channel.id)
        Button(pinned ? "ピン留めを解除" : "ピン留め", systemImage: pinned ? "pin.slash" : "pin") {
            Task { await workspace.setChannelPinned(channel, pinned: !pinned) }
        }
        .disabled(workspace.updatingPin || channel.deleted_at != nil || (!pinned && channel.archive_at != nil))
        .accessibilityIdentifier("toggle-channel-pin")
    }
}

struct PersonRow: View {
    let person: Person
    var body: some View {
        HStack(spacing: 12) {
            AvatarView(name: person.name, url: person.avatarURL)
            VStack(alignment: .leading) {
                Text(person.name)
                Text("@\(person.user_name)").font(.caption).foregroundStyle(.secondary)
            }
            if person.is_bot { Text("アプリ").font(.caption).foregroundStyle(.secondary) }
        }
    }
}
