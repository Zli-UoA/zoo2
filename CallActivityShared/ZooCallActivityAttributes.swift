import ActivityKit
import Foundation

struct ZooCallActivityAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        var ended: Bool
    }

    let startedAt: Date
}
