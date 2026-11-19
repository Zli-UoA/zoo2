import AVFAudio
import SwiftUI

struct CallBanner: View {
    let workspace: WorkspaceStore
    private var calls: CallStore { workspace.calls }
    var body: some View {
        VStack(spacing: 0) {
            if let channel = calls.channel {
                Button { calls.showingCall = true } label: {
                    HStack(spacing: AppDesign.spacing) {
                        Image(systemName: calls.muted ? "mic.slash.fill" : "headphones")
                        Text("\(workspace.name(for: channel))・\(calls.ready ? "通話中" : "接続中")")
                        Spacer()
                        Image(systemName: "chevron.right")
                    }
                    .font(.callout)
                    .padding(.horizontal, AppDesign.horizontalInset)
                    .frame(minHeight: AppDesign.minimumControlSize)
                    .contentShape(Capsule())
                    .glassEffect(.regular.interactive(), in: Capsule())
                }
                .buttonStyle(.plain)
                .padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, 4)
                .accessibilityIdentifier("active-call")
            } else {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    if let invite = calls.invitations.first(where: {
                        (ZooDate.parse($0.expires_at) ?? .distantPast) > context.date &&
                        (workspace.systemCalls?.incomingInvitationID == nil || UUID(uuidString: $0.id) == workspace.systemCalls?.incomingInvitationID)
                    }) {
                        HStack(spacing: AppDesign.spacing) {
                            Label("\(workspace.people[invite.inviter_id]?.name ?? "メンバー")から通話", systemImage: "phone.fill")
                            Spacer()
                            Button("拒否") {
                                if workspace.systemCalls?.decline(invite) != true { Task { await calls.decline(invite) } }
                            }.buttonStyle(.glass).accessibilityIdentifier("incoming-call-decline")
                            Button("参加") {
                                if workspace.systemCalls?.answer(invite) == true { return }
                                Task {
                                    do {
                                        let channel = try await workspace.client.channel(id: invite.channel_id)
                                        try Task.checkCancellation()
                                        calls.start(channel: channel, invitationID: invite.id)
                                    } catch { workspace.report(error) }
                                }
                            }.buttonStyle(.glassProminent).accessibilityIdentifier("incoming-call-answer")
                        }
                        .font(.callout).padding(AppDesign.horizontalInset)
                        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
                        .padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, 4)
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
    @State private var invitingChannel: Channel?

    var body: some View {
        NavigationStack {
            AppForm {
                if let channel = store.channel {
                    Section {
                        Label(workspace.name(for: channel), systemImage: "headphones").font(.headline)
                        if !store.ready { ProgressView("接続中…") }
                        else if store.participants.count <= 1 { Text("ほかのメンバーの参加を待っています。") }
                        Text("ほかのアプリを開いても通話を続けられます。").font(.caption).foregroundStyle(.secondary)
                    }
                    Section("参加者（\(store.participants.count)人）") {
                        ForEach(store.participants) { person in
                            HStack(spacing: AppDesign.spacing) {
                                AvatarView(name: person.name, url: workspace.people[person.user_id]?.avatarURL)
                                Text(person.name)
                                Spacer()
                                if person.is_muted { Image(systemName: "mic.slash.fill").accessibilityLabel("ミュート中") }
                            }
                        }
                    }
                    if !channel.is_dm {
                        Section("招待") {
                            Button("メンバーを招待", systemImage: "person.badge.plus") { invitingChannel = channel }
                                .disabled(!store.ready).accessibilityIdentifier("call-invite")
                            TimelineView(.periodic(from: .now, by: 1)) { context in
                                ForEach(store.pendingOutgoing(at: context.date)) { invitation in
                                    HStack(spacing: AppDesign.spacing) {
                                        Text("\(workspace.people[invitation.invitee_id]?.name ?? "メンバー")さんを呼び出し中…")
                                        Spacer()
                                        Button("取り消す") { Task { await store.cancelInvitation(invitation) } }
                                            .disabled(!store.ready || store.cancellingInvitations.contains(invitation.id))
                                            .accessibilityIdentifier("call-cancel-\(invitation.invitee_id)")
                                    }
                                }
                            }
                            if let notice = store.invitationNotice { Text(notice).foregroundStyle(.secondary) }
                            if let error = store.invitationError { Text(error).foregroundStyle(.red) }
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
            .toolbar { ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } } }
            .sheet(item: $invitingChannel) { channel in
                CallInviteView(store: store, workspace: workspace, channel: channel)
            }
            .onChange(of: store.ready) { _, ready in if !ready { invitingChannel = nil } }
        }
    }
}

private struct CallInviteView: View {
    let store: CallStore
    let workspace: WorkspaceStore
    let channel: Channel
    @Environment(\.dismiss) private var dismiss
    @State private var selected: Set<String> = []

    var body: some View {
        NavigationStack {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                let members = workspace.channelMembers[channel.id] ?? []
                let candidates = store.invitationCandidates(members: members, at: context.date)
                let chosen = selected.intersection(candidates.map(\.id))
                let slots = store.invitationSlots(at: context.date)
                AppList {
                    Section {
                        Text("\(channel.channel_name)の通話に招待します。")
                        Text(slots == 0 ? "通話は満員です。" : "あと\(slots)人まで招待できます。")
                            .foregroundStyle(.secondary)
                    }
                    Section("メンバー") {
                        if let error = workspace.memberErrors[channel.id] {
                            Text(error).foregroundStyle(.red)
                            Button("再読み込み") { Task { await workspace.loadMembers(channelID: channel.id) } }
                        } else if workspace.channelMembers[channel.id] == nil {
                            ProgressView("メンバーを読み込み中…")
                        } else if candidates.isEmpty {
                            Text("招待できるメンバーはいません。")
                        } else {
                            ForEach(candidates) { person in
                                Button {
                                    if selected.contains(person.id) { selected.remove(person.id) }
                                    else { selected.insert(person.id) }
                                } label: {
                                    HStack(spacing: AppDesign.spacing) {
                                        PersonRow(person: person)
                                        Spacer()
                                        Image(systemName: chosen.contains(person.id) ? "checkmark.circle.fill" : "circle")
                                    }.contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .disabled(store.inviting || (!chosen.contains(person.id) && chosen.count >= slots))
                                .accessibilityLabel(person.name)
                                .accessibilityValue(chosen.contains(person.id) ? "選択済み" : "未選択")
                                .accessibilityIdentifier("call-invite-member-\(person.id)")
                            }
                        }
                    }
                    if let error = store.invitationError {
                        Section { Text(error).foregroundStyle(.red).accessibilityIdentifier("call-invite-error") }
                    }
                }
                .onChange(of: candidates.map(\.id)) { _, ids in selected.formIntersection(ids) }
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        AppCloseButton { dismiss() }.accessibilityIdentifier("call-invite-close")
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button(store.inviting ? "送信中…" : "招待する") {
                            Task {
                                if await store.invite(userIDs: chosen, members: members, channelID: channel.id) { dismiss() }
                            }
                        }
                        .disabled(!store.ready || store.inviting || chosen.isEmpty || chosen.count > slots || workspace.memberErrors[channel.id] != nil)
                        .accessibilityIdentifier("call-send-invitations")
                    }
                }
            }
            .navigationTitle("メンバーを招待").navigationBarTitleDisplayMode(.inline)
            .task { await workspace.loadMembers(channelID: channel.id) }
        }
    }
}
