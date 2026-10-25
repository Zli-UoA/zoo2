import Foundation
import Observation

@Observable @MainActor
final class NotificationsViewModel {
    private let repository: any WorkspaceRepository
    var onlyUnread = false
    init(repository: any WorkspaceRepository) { self.repository = repository }
    var activities: [Activity] { repository.state.activities.filter { !onlyUnread || !$0.is_read } }
    var pendingCount: Int { repository.state.activities.filter { !$0.is_read }.count }
    func read(_ activity: Activity) async throws { try await repository.readActivity(activity) }
}
