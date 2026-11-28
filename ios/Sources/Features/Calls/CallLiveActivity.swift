import ActivityKit
import Foundation

@MainActor
protocol CallLiveActivity: AnyObject {
    func start(onEnded: @escaping @MainActor () -> Void) throws
    func finish()
}

@MainActor
final class NativeCallLiveActivity: CallLiveActivity {
    private var activityID: String?
    private var observation: Task<Void, Never>?

    func start(onEnded: @escaping @MainActor () -> Void) throws {
        guard ActivityAuthorizationInfo().areActivitiesEnabled else {
            throw ZooError.auth("Siriからの通話には、設定でZooのライブアクティビティを許可してください。")
        }
        do {
            let activity = try Activity.request(attributes: ZooCallActivityAttributes(startedAt: Date()),
                                                content: .init(state: .init(ended: false), staleDate: nil))
            activityID = activity.id
            observation = Task {
                for await state in activity.activityStateUpdates {
                    guard !Task.isCancelled else { return }
                    if state == .dismissed || state == .ended {
                        onEnded()
                        return
                    }
                }
            }
        } catch {
            throw ZooError.auth("通話中の表示を開始できませんでした。Zooを開いて発信してください。")
        }
    }

    func finish() {
        observation?.cancel()
        observation = nil
        guard let activityID else { return }
        self.activityID = nil
        Self.end(ids: [activityID])
    }

    static func endInterrupted() {
        // 起動時点の表示だけを終了し、この後に始まる通話を巻き込まない。
        end(ids: Set(Activity<ZooCallActivityAttributes>.activities.map(\.id)))
    }

    private static func end(ids: Set<String>) {
        // ActivityはSendableではないため、終了タスク内で対象を取得する。
        Task { @concurrent in
            for activity in Activity<ZooCallActivityAttributes>.activities where ids.contains(activity.id) {
                await activity.end(.init(state: .init(ended: true), staleDate: nil), dismissalPolicy: .immediate)
            }
        }
    }
}
