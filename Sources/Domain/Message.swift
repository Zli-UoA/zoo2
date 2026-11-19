import Foundation

struct Message: Codable, Identifiable, Hashable, Sendable {
    let id: String
    var user_id: String
    var channel_id: String
    var content: String
    var created_at: String
    var updated_at: String?
    var deleted_at: String?
    var parent_id: String?
    var card: String?
    var display_data: MessageDisplayData?

    var date: Date? { ZooDate.parse(created_at) }
}
