import SwiftUI

struct ChannelDetailsView: View {
    let channel: Channel
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var requestedPin: Bool?
    @State private var editing = false
    @State private var confirmingArchive = false
    @State private var archiving = false
    @State private var error: String?

    private var pinned: Bool { workspace.pinnedChannelIDs.contains(channel.id) }
    private var currentChannel: Channel { workspace.channels.first { $0.id == channel.id } ?? channel }

    var body: some View {
        let channel = currentChannel
        NavigationStack {
            AppForm {
                Section("会話について") {
                    Label(workspace.name(for: channel), systemImage: channel.symbol).font(.headline)
                    if let description = channel.description, !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        Text(description).textSelection(.enabled)
                    } else {
                        LabeledContent("説明", value: "なし")
                    }
                    LabeledContent("種類", value: channel.is_dm ? "ダイレクトメッセージ" : channel.is_private ? "非公開チャンネル" : "公開チャンネル")
                    if channel.archive_at != nil { Label("アーカイブ済み", systemImage: "archivebox") }
                    if channel.encrypted == true { Label("暗号化された会話", systemImage: "lock.shield") }
                    Toggle("ピン留め", isOn: Binding(get: { requestedPin ?? pinned }, set: { value in
                        requestedPin = value
                        Task {
                            await workspace.setChannelPinned(channel, pinned: value)
                            requestedPin = nil
                        }
                    }))
                    .disabled(requestedPin != nil || workspace.updatingPin || channel.deleted_at != nil || (!pinned && channel.archive_at != nil))
                    .accessibilityIdentifier("channel-pin-toggle")
                    if requestedPin != nil || workspace.updatingPin { ProgressView("保存中…") }
                    if let error = workspace.pinUpdateError {
                        Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.red)
                    }
                }
                if channel.canManage(by: workspace.user) {
                    Section("チャンネルの管理") {
                        Button("名前と説明を編集", systemImage: "pencil") { editing = true }
                            .accessibilityIdentifier("edit-channel")
                        Button("チャンネルをアーカイブ", systemImage: "archivebox", role: .destructive) { confirmingArchive = true }
                            .accessibilityIdentifier("archive-channel")
                            .confirmationDialog("このチャンネルをアーカイブしますか？", isPresented: $confirmingArchive, titleVisibility: .visible) {
                                Button("アーカイブ", role: .destructive) { archiving = true }
                            } message: {
                                Text("新しい投稿と通話を停止します。過去のメッセージは引き続き閲覧できます。")
                            }
                    }
                    .disabled(workspace.updatingChannels.contains(channel.id))
                }
                if let error { Text(error).foregroundStyle(.red).accessibilityIdentifier("channel-management-error") }
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
            .toolbar { ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() }.disabled(archiving) } }
            .task { await workspace.loadMembers(channelID: channel.id) }
            .sheet(isPresented: $editing) { ChannelEditor(channel: currentChannel, workspace: workspace) }
            .task(id: archiving) {
                guard archiving else { return }
                defer { archiving = false }
                do {
                    try await workspace.archiveChannel(channel)
                    dismiss()
                } catch {
                    if !Task.isCancelled { self.error = error.localizedDescription }
                    if case ZooError.expiredSession = error { workspace.report(error) }
                }
            }
            .interactiveDismissDisabled(archiving)
        }
    }
}

private struct ChannelEditor: View {
    let channel: Channel
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var name: String
    @State private var description: String
    @State private var saving = false
    @State private var error: String?

    init(channel: Channel, workspace: WorkspaceStore) {
        self.channel = channel
        self.workspace = workspace
        _name = State(initialValue: channel.channel_name)
        _description = State(initialValue: channel.description ?? "")
    }

    private var trimmedName: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var valid: Bool {
        let name = trimmedName
        let unchangedName = name == channel.channel_name
        let validName = !name.isEmpty && name.unicodeScalars.count <= 80 &&
            !["all", "channel", "everyone", "here", "general"].contains(name) &&
            name.range(of: #"\A[\p{L}\p{M}\p{N}_-]+\z"#, options: .regularExpression) != nil
        return (unchangedName || validName) && description.unicodeScalars.count <= 250 &&
            (name != channel.channel_name || description != (channel.description ?? ""))
    }

    var body: some View {
        NavigationStack {
            AppForm {
                Section {
                    TextField("チャンネル名", text: $name).textInputAutocapitalization(.never).autocorrectionDisabled()
                        .accessibilityIdentifier("edit-channel-name")
                    TextField("説明", text: $description, axis: .vertical).lineLimit(3...6)
                        .accessibilityIdentifier("edit-channel-description")
                } footer: {
                    Text("名前は80文字以内の文字・数字・ハイフン・アンダースコア、説明は250文字以内で入力してください。")
                }
                if let error { Text(error).foregroundStyle(.red).accessibilityIdentifier("channel-edit-error") }
            }
            .disabled(saving)
            .navigationTitle("チャンネルを編集").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("キャンセル") { dismiss() }.disabled(saving) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("保存") { saving = true }.disabled(!valid || saving)
                        .accessibilityIdentifier("save-channel")
                }
            }
            .task(id: saving) {
                guard saving else { return }
                defer { saving = false }
                do {
                    try await workspace.updateChannel(channel, name: trimmedName, description: description)
                    dismiss()
                } catch {
                    if !Task.isCancelled { self.error = error.localizedDescription }
                    if case ZooError.expiredSession = error { workspace.report(error) }
                }
            }
            .interactiveDismissDisabled(saving)
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
        HStack(spacing: AppDesign.spacing) {
            AvatarView(name: person.name, url: person.avatarURL)
            VStack(alignment: .leading, spacing: 2) {
                Text(person.name)
                Text("@\(person.user_name)").font(.caption).foregroundStyle(.secondary)
            }
            if person.is_bot { Text("アプリ").font(.caption).foregroundStyle(.secondary) }
        }
    }
}
