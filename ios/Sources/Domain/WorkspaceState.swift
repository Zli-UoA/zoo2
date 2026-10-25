import Foundation
import Observation

@Observable @MainActor
final class WorkspaceState {
    var me: Person?
    var people: [Person] = []
    var channels: [Channel] = []
    var memberships: [Membership] = []
    var activities: [Activity] = []
    var unread: Set<String> = []
    var channelPhase: SyncPhase = .idle
    var syncErrors: [String: String] = [:]

    func reset() {
        me = nil
        people = []; channels = []; memberships = []; activities = []; unread = []
        channelPhase = .idle
        syncErrors = [:]
    }
}
