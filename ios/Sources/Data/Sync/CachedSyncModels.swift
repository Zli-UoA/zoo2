import Foundation
import SwiftData

@Model
final class CachedSyncRecord {
    @Attribute(.unique) var id: String
    var account: String
    var resource: String
    var channelID: String?
    var payload: Data

    init(account: String, resource: String, channelID: String?, payload: Data) {
        id = account + "|" + resource
        self.account = account
        self.resource = resource
        self.channelID = channelID
        self.payload = payload
    }
}

@Model
final class CachedSyncAccount {
    @Attribute(.unique) var connection: String
    var account: String
    var profile: Data

    init(connection: String, account: String, profile: Data) {
        self.connection = connection
        self.account = account
        self.profile = profile
    }
}
