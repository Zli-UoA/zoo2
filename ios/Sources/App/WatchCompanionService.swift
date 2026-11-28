import Foundation

@MainActor
final class WatchCompanionService {
    private let currentWorkspace: @MainActor () -> WorkspaceStore?
    private let prepareWorkspace: @MainActor () async -> WorkspaceStore?
    private let refreshNotices: @MainActor (WorkspaceStore) async -> Void
    private let calls: SystemCallController
    private weak var workspace: WorkspaceStore?
    private var sessionID = UUID()
    private var performingAction = false
    private var refreshTask: Task<Void, Never>?
    private var refreshID = UUID()

    convenience init(session: AppSession, calls: SystemCallController) {
        self.init(calls: calls, workspace: { session.workspace }, prepare: { await session.prepareForIncomingCall() })
    }

    init(calls: SystemCallController, workspace: @escaping @MainActor () -> WorkspaceStore?,
         prepare: @escaping @MainActor () async -> WorkspaceStore?,
         refresh: @escaping @MainActor (WorkspaceStore) async -> Void = { await $0.inbox.refresh(clearActionError: false) }) {
        currentWorkspace = workspace
        prepareWorkspace = prepare
        refreshNotices = refresh
        self.calls = calls
    }

    deinit { refreshTask?.cancel() }

    func snapshot() -> WatchSnapshot {
        let current = currentWorkspace()
        if workspace !== current {
            refreshID = UUID()
            refreshTask?.cancel()
            refreshTask = nil
            workspace = current
            sessionID = UUID()
        }
        guard let workspace, workspace.active else { return WatchSnapshot() }
        let notices = workspace.inbox.entries.prefix(10).map { entry in
            let actor = entry.actorName ?? entry.actorIDs.first.flatMap { workspace.people[$0]?.name } ?? "メンバー"
            let title: String
            switch entry.kind {
            case "dm": title = "ダイレクトメッセージ"
            case "mention": title = "メンション"
            case "reply": title = "返信"
            case "channel_message": title = "チャンネルのメッセージ"
            case "reaction", "like": title = "リアクション"
            case "missed_call": title = "不在着信"
            case "follow_request": title = "フォロー申請"
            case "follow_accepted": title = "フォローが承認されました"
            case "repost": title = "リポスト"
            default: title = "通知"
            }
            // Watchには本文や認証情報を複製せず、通知の種類と送信者だけを渡す。
            return WatchNotice(id: entry.id, title: title, detail: String(actor.prefix(80)),
                               date: entry.createdAt, unread: entry.isUnread)
        }
        return WatchSnapshot(sessionID: sessionID, call: calls.watchCall, notices: notices,
                             unreadCount: workspace.inbox.unreadCount, noticesLoading: workspace.inbox.loading,
                             noticeError: workspace.inbox.error, callError: workspace.calls.error)
    }

    func respond(to request: WatchRequest) async -> WatchReply {
        guard request.version == 1 else {
            return WatchReply(snapshot: WatchSnapshot(), error: "iPhoneとWatchのZooを更新してください。")
        }
        if request.action == .refresh {
            if currentWorkspace() == nil { _ = await prepareWorkspace() }
            let current = snapshot()
            if let workspace, workspace.active, refreshTask == nil {
                let id = refreshID
                let refreshNotices = refreshNotices
                // 通知APIの遅延で着信への応答を妨げず、同じ取得を重ねて起動しない。
                refreshTask = Task { [weak self] in
                    await refreshNotices(workspace)
                    guard self?.refreshID == id else { return }
                    self?.refreshTask = nil
                }
            }
            return WatchReply(snapshot: current)
        }
        let current = snapshot()
        guard let context = current.sessionID, request.sessionID == context,
              let call = current.call, request.callID == call.id else {
            return WatchReply(snapshot: current, error: "通話の状態が変わりました。再読み込みしてください。")
        }
        guard !performingAction else {
            return WatchReply(snapshot: current, error: "前の操作を処理中です。")
        }
        performingAction = true
        defer { performingAction = false }
        do {
            try await calls.performWatchAction(request)
            return WatchReply(snapshot: snapshot())
        } catch {
            return WatchReply(snapshot: snapshot(), error: "操作できませんでした。iPhoneで通話の状態を確認してください。")
        }
    }
}
