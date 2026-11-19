import Foundation

struct SyncLease: Sendable {
    let account: String
    let generation: UUID
}

enum SyncInvalidation: Error { case notifications }
