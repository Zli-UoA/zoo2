import Foundation

struct IncomingCall: Equatable, Sendable {
    let id: UUID
    let userID: String
    let channelID: String
    let inviterID: String
    let callerName: String
    let expiresAt: Date

    init?(payload: [AnyHashable: Any]) {
        guard payload["type"] as? String == "call_invite",
              let id = payload["invite_id"] as? String, let uuid = UUID(uuidString: id),
              let user = payload["user_id"] as? String, UUID(uuidString: user) != nil,
              let channel = payload["channel_id"] as? String, UUID(uuidString: channel) != nil,
              let inviter = payload["inviter_id"] as? String, UUID(uuidString: inviter) != nil,
              let expiry = payload["expires_at"] as? String, let date = ZooDate.parse(expiry) else { return nil }
        self.id = uuid; userID = user; channelID = channel; inviterID = inviter
        callerName = String((payload["caller_name"] as? String ?? "Zooの通話").prefix(120))
        expiresAt = date
    }

    init?(invitation: CallInvitation, callerName: String) {
        self.init(payload: ["type": "call_invite", "invite_id": invitation.id, "user_id": invitation.invitee_id,
                            "channel_id": invitation.channel_id, "inviter_id": invitation.inviter_id,
                            "caller_name": callerName, "expires_at": invitation.expires_at])
    }

    func canAnswer(userID: String, at date: Date = .now) -> Bool {
        self.userID.caseInsensitiveCompare(userID) == .orderedSame && expiresAt > date
    }

    var invitation: CallInvitation {
        CallInvitation(id: id.uuidString.lowercased(), channel_id: channelID, inviter_id: inviterID,
                       invitee_id: userID, status: "pending", expires_at: expiresAt.ISO8601Format())
    }
}
