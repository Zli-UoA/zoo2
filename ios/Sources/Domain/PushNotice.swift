import Foundation

struct PushDestination: Equatable, Sendable {
    let userID: String
    let channelID: String
    let messageID: String?
    init?(payload: [AnyHashable: Any]) {
        guard payload["type"] as? String != "push_test",
              let user = payload["user_id"] as? String, UUID(uuidString: user) != nil,
              let channel = payload["channel_id"] as? String, UUID(uuidString: channel) != nil else { return nil }
        let message = payload["message_id"] as? String
        if let value = payload["message_id"], !(value is NSNull) {
            guard let message, UUID(uuidString: message) != nil else { return nil }
        }
        userID = user; channelID = channel; messageID = message
    }
}

enum PushNotice: Equatable, Sendable {
    case message(PushDestination)
    case test(userID: String, id: UUID)

    init?(payload: [AnyHashable: Any]) {
        if payload["type"] as? String == "push_test" {
            guard let user = payload["user_id"] as? String, UUID(uuidString: user) != nil,
                  let id = payload["test_id"] as? String, let testID = UUID(uuidString: id) else { return nil }
            self = .test(userID: user, id: testID)
        } else {
            guard let destination = PushDestination(payload: payload) else { return nil }
            self = .message(destination)
        }
    }

    var userID: String {
        switch self {
        case .message(let destination): destination.userID
        case .test(let userID, _): userID
        }
    }
}
