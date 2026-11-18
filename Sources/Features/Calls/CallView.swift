import AVFAudio
import SwiftUI

struct CallBanner: View {
    let workspace: WorkspaceStore
    private var calls: CallStore { workspace.calls }
    var body: some View {
        VStack(spacing: 0) {
            if let channel = calls.channel {
                Button { calls.showingCall = true } label: {
                    HStack {
                        Image(systemName: calls.muted ? "mic.slash.fill" : "headphones")
                        Text("\(workspace.name(for: channel))・\(calls.ready ? "通話中" : "接続中")")
                        Spacer()
                        Image(systemName: "chevron.right")
                    }.padding(12).background(.green.opacity(0.12))
                }.accessibilityIdentifier("active-call")
            } else {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    if let invite = calls.invitations.first(where: { (ZooDate.parse($0.expires_at) ?? .distantPast) > context.date }) {
                        HStack {
                            Label("\(workspace.people[invite.inviter_id]?.name ?? "メンバー")から通話", systemImage: "phone.fill")
                            Spacer()
                            Button("辞退") { Task { await calls.decline(invite) } }
                            Button("応答") {
                                Task {
                                    do {
                                        let channel = try await workspace.client.channel(id: invite.channel_id)
                                        try Task.checkCancellation()
                                        calls.start(channel: channel, invitationID: invite.id)
                                    } catch { workspace.report(error) }
                                }
                            }.buttonStyle(.borderedProminent)
                        }.font(.callout).padding(12).background(.green.opacity(0.12))
                    }
                }
            }
        }
    }
}

struct CallView: View {
    @Bindable var store: CallStore
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                if let channel = store.channel {
                    Section {
                        Label(workspace.name(for: channel), systemImage: "headphones").font(.title2.bold())
                        if !store.ready { ProgressView("接続中…") }
                        else if store.participants.count <= 1 { Text("ほかのメンバーの参加を待っています。") }
                        Text("アプリを離れると通話を終了します。").font(.caption).foregroundStyle(.secondary)
                    }
                    Section("参加者（\(store.participants.count)人）") {
                        ForEach(store.participants) { person in
                            HStack {
                                AvatarView(name: person.name, url: workspace.people[person.user_id]?.avatarURL)
                                Text(person.name)
                                Spacer()
                                if person.is_muted { Image(systemName: "mic.slash.fill").accessibilityLabel("ミュート中") }
                            }
                        }
                    }
                    Section {
                        Button(store.muted ? "ミュートを解除" : "ミュート", systemImage: store.muted ? "mic.slash" : "mic") { store.toggleMute() }
                            .disabled(!store.ready || store.changingMute).accessibilityIdentifier("call-mute")
                        Button(store.speakerEnabled ? "スピーカーをオフ" : "スピーカーをオン", systemImage: "speaker.wave.2") { store.toggleSpeaker() }
                            .disabled(!store.ready)
                        Button("通話を終了", systemImage: "phone.down.fill", role: .destructive) { store.finish(); dismiss() }
                            .accessibilityIdentifier("call-end")
                    }
                } else if store.error == nil { Text("通話は終了しました。") }
                if let error = store.error { Text(error).foregroundStyle(.red).accessibilityIdentifier("call-error") }
            }
            .navigationTitle("音声通話").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("閉じる") { dismiss() } } }
        }
    }
}
