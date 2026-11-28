import AVFAudio
import SwiftUI

struct CallBanner: View {
    let workspace: WorkspaceStore
    @State private var joinTask: Task<Void, Never>?
    @State private var joinID = UUID()
    @State private var joining = false
    @State private var declining = false
    private var calls: CallStore { workspace.calls }
    var body: some View {
        VStack(spacing: 0) {
            if let channel = calls.channel {
                Button { calls.showingCall = true } label: {
                    HStack(spacing: AppDesign.spacing) {
                        Image(systemName: calls.muted ? "mic.slash.fill" : "headphones")
                        Text("\(workspace.name(for: channel))・\(calls.connectionStatus)")
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
            } else if !calls.invitations.isEmpty {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    if let invite = calls.invitations.first(where: {
                        (ZooDate.parse($0.expires_at) ?? .distantPast) > context.date &&
                        (workspace.systemCalls?.canPresentInvitation($0, at: context.date) ?? true)
                    }) {
                        HStack(spacing: AppDesign.spacing) {
                            Label("\(workspace.people[invite.inviter_id]?.name ?? "メンバー")から通話", systemImage: "phone.fill")
                            Spacer()
                            Button("拒否") {
                                joinID = UUID()
                                joinTask?.cancel()
                                joining = false
                                if workspace.systemCalls?.decline(invite) != true {
                                    declining = true
                                    Task { await calls.decline(invite); declining = false }
                                }
                            }.disabled(declining).buttonStyle(.glass).accessibilityIdentifier("incoming-call-decline")
                            Button("参加") {
                                if workspace.systemCalls?.answer(invite) == true { return }
                                let operation = UUID()
                                joinID = operation
                                joining = true
                                joinTask = Task {
                                    defer { if joinID == operation { joining = false; joinTask = nil } }
                                    do {
                                        let channel = try await workspace.client.channel(id: invite.channel_id)
                                        try Task.checkCancellation()
                                        guard joinID == operation, calls.invitations.contains(where: { $0.id == invite.id }),
                                              (ZooDate.parse(invite.expires_at) ?? .distantPast) > .now else { return }
                                        calls.start(channel: channel, invitationID: invite.id)
                                    } catch { if !Task.isCancelled { workspace.report(error) } }
                                }
                            }.disabled(joining || declining).buttonStyle(.glassProminent)
                                .tint(AppDesign.primary).foregroundStyle(AppDesign.primaryForeground)
                                .accessibilityIdentifier("incoming-call-answer")
                        }
                        .font(.callout).padding(AppDesign.horizontalInset)
                        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
                        .padding(.horizontal, AppDesign.horizontalInset).padding(.vertical, 4)
                    }
                }
            }
        }
        .onDisappear { joinID = UUID(); joinTask?.cancel(); joinTask = nil; joining = false }
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
                        Text(store.connectionStatus).accessibilityIdentifier("call-connection-status")
                        if !store.ready { ProgressView() }
                        else if store.participants.count <= 1 { Text("ほかのメンバーの参加を待っています。") }
                        Text("ほかのアプリを開いても通話を続けられます。").font(.caption).foregroundStyle(.secondary)
                    }
                    Section("参加者（\(store.participants.count)人）") {
                        ForEach(store.participants) { person in
                            HStack(spacing: AppDesign.spacing) {
                                AvatarView(name: person.name, url: workspace.people[person.user_id]?.avatarURL)
                                Text(person.name)
                                if let state = store.peerStates[person.id], state != .connected {
                                    Text(state.label).font(.caption).foregroundStyle(.secondary)
                                }
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
                    CallDiagnosticsSection(store: store)
                } else if store.error == nil { Text("通話は終了しました。") }
                if let error = store.error { Text(error).foregroundStyle(.red).accessibilityIdentifier("call-error") }
                if store.retryChannel != nil {
                    Button("再接続", systemImage: "arrow.clockwise") { store.retry() }
                        .accessibilityIdentifier("call-retry")
                }
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

private struct CallDiagnosticsSection: View {
    let store: CallStore
    @State private var expanded = false

    var body: some View {
        Section {
            DisclosureGroup("通話の通信状況", isExpanded: $expanded) {
                if expanded {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("通話情報の接続：\(store.ready ? "接続済み" : "未接続")")
                            .accessibilityIdentifier("call-diagnostics-server")
                        Text("接続サーバー：\(store.iceServerKinds.isEmpty ? "なし" : store.iceServerKinds.joined(separator: "・"))")
                            .accessibilityIdentifier("call-diagnostics-ice")
                        ForEach(store.diagnostics) { peer in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(store.participants.first { $0.id == peer.id }?.name ?? "参加者").bold()
                                Text("\(peer.state.label)・ICE：\(peer.iceState)")
                                Text("経路：\(peer.route)")
                                Text("音声受信：\(peer.receivedBytes) B・送信：\(peer.sentBytes) B")
                            }
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .font(.caption)
                    .task {
                        // 通信状況を開いている間だけ統計を取得し、閉じたら停止する。
                        while !Task.isCancelled {
                            await store.refreshDiagnostics()
                            do { try await Task.sleep(for: .seconds(2)) } catch { return }
                        }
                    }
                }
            }
            .accessibilityIdentifier("call-diagnostics")
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
                            ProgressView("メンバーを読み込み中…").frame(maxWidth: .infinity)
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
