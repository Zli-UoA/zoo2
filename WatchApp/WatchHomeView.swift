import SwiftUI
import WatchKit

struct WatchHomeView: View {
    let state: WatchCompanionState
    let send: (WatchAction, Bool?) -> Void

    private var incomingCallID: UUID? {
        guard let call = state.snapshot?.call, call.allows(.answer) else { return nil }
        return call.id
    }

    var body: some View {
        NavigationStack {
            List {
                if !state.reachable {
                    Section {
                        Label("iPhoneに接続できません", systemImage: "iphone.slash")
                        Text("近くに置いてZooを開いてください。")
                            .font(.footnote).foregroundStyle(.secondary)
                    }
                } else if let snapshot = state.snapshot {
                    if snapshot.sessionID == nil {
                        Text("iPhoneのZooでログインしてください。")
                    } else {
                        callSection(snapshot.call)
                        Section("通知 · 未読\(snapshot.unreadCount)件") {
                            if let error = snapshot.noticeError {
                                Text(error).font(.footnote).foregroundStyle(.secondary)
                            }
                            if snapshot.notices.isEmpty {
                                if snapshot.noticesLoading { ProgressView("通知を読み込み中…") }
                                else { Text("通知はありません") }
                            }
                            ForEach(snapshot.notices) { notice in
                                VStack(alignment: .leading, spacing: 4) {
                                    Label(notice.title, systemImage: notice.unread ? "circle.fill" : "circle")
                                        .font(.headline)
                                    Text(notice.detail).font(.subheadline)
                                    Text(notice.date, style: .relative)
                                        .font(.caption2).foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
                if state.pending != nil { ProgressView("iPhoneに接続中…") }
                if let error = state.error {
                    Text(error).font(.footnote).foregroundStyle(.orange)
                }
                if let notice = state.notice {
                    Text(notice).font(.footnote).foregroundStyle(.secondary)
                }
                Button("再読み込み", systemImage: "arrow.clockwise") { send(.refresh, nil) }
                    .disabled(!state.reachable || state.pending != nil)
                Section("通知の受け取り方") {
                    Text("iPhoneのWatchアプリで「通知」からZooの通知を許可してください。本文の表示は通知設定に従います。")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Zoo")
            .privacySensitive()
        }
        .onChange(of: incomingCallID) { _, current in
            if current != nil { WKInterfaceDevice.current().play(.notification) }
        }
    }

    @ViewBuilder
    private func callSection(_ call: WatchCall?) -> some View {
        Section("iPhoneの通話") {
            if let call {
                Text(call.title).font(.headline)
                Text(call.status).foregroundStyle(.secondary)
                Text("音声はiPhoneから流れます。")
                    .font(.footnote).foregroundStyle(.secondary)
                if call.phase == .incoming {
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        VStack {
                            Button("iPhoneで応答", systemImage: "phone.fill") { send(.answer, nil) }
                                .tint(.green)
                            Button("拒否", systemImage: "phone.down.fill", role: .destructive) { send(.decline, nil) }
                        }
                        .buttonStyle(.bordered)
                        .disabled(state.pending != nil || !call.allows(.answer, at: context.date))
                    }
                } else {
                    Button(call.muted ? "ミュート解除" : "ミュート", systemImage: call.muted ? "mic.slash.fill" : "mic.fill") {
                        send(.setMuted, !call.muted)
                    }
                    .disabled(state.pending != nil || !call.canSetMuted)
                    Button("通話を終了", systemImage: "phone.down.fill", role: .destructive) { send(.end, nil) }
                        .disabled(state.pending != nil)
                }
            } else {
                Label("現在の通話はありません", systemImage: "phone")
            }
        }
    }
}
