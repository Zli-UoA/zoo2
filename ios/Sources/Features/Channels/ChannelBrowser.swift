import SwiftUI

struct ChannelBrowser: View {
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""
    @State private var includeArchived = false
    @State private var opening: Channel?
    @State private var error: String?

    private var matches: [Channel] {
        workspace.channels.filter { channel in
            guard !channel.is_dm,
                  search.isEmpty || channel.channel_name.localizedStandardContains(search) else { return false }
            let joined = workspace.joinedChannelIDs.contains(channel.id)
            if channel.archive_at != nil { return includeArchived && (!channel.is_private || joined) }
            return !channel.is_private && !joined
        }
    }

    var body: some View {
        NavigationStack {
            AppList {
                Toggle("アーカイブ済みを含める", isOn: $includeArchived)
                if let error { Text(error).foregroundStyle(.red) }
                if matches.isEmpty { Text("該当するチャンネルはありません").foregroundStyle(.secondary) }
                ForEach(matches) { channel in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(channel.channel_name)
                            if let description = channel.description { Text(description).font(.caption).foregroundStyle(.secondary) }
                        }
                        Spacer()
                        Button(channel.archive_at == nil ? "参加" : "開く") { opening = channel }
                            .buttonStyle(.bordered).buttonBorderShape(.capsule).controlSize(.small)
                            .accessibilityIdentifier("browse-channel-\(channel.id)")
                    }
                }
            }
            .disabled(opening != nil)
            .navigationTitle("チャンネルを探す")
            .navigationBarTitleDisplayMode(.inline)
            .appSearch(text: $search, prompt: "チャンネル名を検索", identifier: "channel-browser")
            .toolbar { ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } } }
            .task(id: opening?.id) {
                guard let channel = opening else { return }
                defer { opening = nil }
                do {
                    if channel.archive_at == nil { try await workspace.client.joinChannel(id: channel.id) }
                    try Task.checkCancellation()
                    workspace.addAndOpen(channel, joined: channel.archive_at == nil)
                    dismiss()
                } catch {
                    if !Task.isCancelled { self.error = error.localizedDescription; workspace.report(error) }
                }
            }
        }
    }
}
