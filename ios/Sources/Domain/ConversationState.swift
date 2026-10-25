import Foundation
import Observation

@Observable @MainActor
final class ConversationState {
    let channelID: String
    private(set) var timeline: [Message] = []
    private(set) var replyCounts: [String: Int] = [:]
    var reactions: [Reaction] = []
    var phase: SyncPhase = .idle
    var reactionError: String?
    private var confirmed: [Message] = []
    private var outgoing: [String: Message] = [:]

    init(channelID: String) { self.channelID = channelID }

    func receive(_ messages: [Message]) {
        confirmed = messages.filter { $0.channel_id == channelID }
        let ids = Set(confirmed.map(\.id))
        outgoing = outgoing.filter { !ids.contains($0.key) }
        rebuild()
    }

    func acknowledge(_ message: Message) {
        guard message.channel_id == channelID else { return }
        if !confirmed.contains(where: { $0.id == message.id }) { outgoing[message.id] = message }
        rebuild()
    }

    func releaseReceivedHistory() {
        confirmed = []
        reactions = []
        rebuild()
    }

    func clear() {
        confirmed = []; outgoing = [:]; reactions = []; reactionError = nil
        rebuild()
    }

    private func rebuild() {
        var all = Dictionary(confirmed.map { ($0.id, $0) }, uniquingKeysWith: { _, latest in latest })
        all.merge(outgoing) { _, latest in latest }
        let dated: [(message: Message, date: Date)] = all.values.map {
            (message: $0, date: ZooDate.parse($0.created_at) ?? Date.distantPast)
        }
        let sorted = dated.sorted { lhs, rhs in
            if lhs.date == rhs.date { return lhs.message.id < rhs.message.id }
            return lhs.date < rhs.date
        }
        timeline = sorted.map(\.message)
        replyCounts = [:]
        for message in timeline where message.deleted_at == nil {
            if let parent = message.parent_id { replyCounts[parent, default: 0] += 1 }
        }
    }
}
