import Foundation

struct ActiveCallSummary: Decodable, Sendable {
    let channel_id: String
    let participant_count: Int
}

struct CallParticipant: Decodable, Identifiable, Sendable {
    let session_id: String
    let user_id: String
    let user_name: String
    let display_name: String
    let is_muted: Bool
    var id: String { session_id }
    var name: String { display_name.isEmpty ? user_name : display_name }
}

struct CallICEServer: Decodable, Sendable {
    let urls: [String]
    let username: String?
    let credential: String?
    private enum CodingKeys: String, CodingKey { case urls, username, credential }
    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        if let url = try? values.decode(String.self, forKey: .urls) { urls = [url] }
        else { urls = try values.decode([String].self, forKey: .urls) }
        username = try values.decodeIfPresent(String.self, forKey: .username)
        credential = try values.decodeIfPresent(String.self, forKey: .credential)
    }
}

struct CallRoom: Decodable, Sendable {
    let participants: [CallParticipant]
    let max_participants: Int
    let ice_servers: [CallICEServer]
}

struct CallSignal: Codable, Equatable, Sendable {
    struct Candidate: Codable, Equatable, Sendable {
        let candidate: String
        let sdpMid: String?
        let sdpMLineIndex: Int32
    }
    let type: String
    var sdp: String? = nil
    var candidate: Candidate? = nil
    var signal_id: String? = nil

    func json() throws -> JSONValue {
        let data = try JSONEncoder().encode(self)
        guard data.count <= 6_500 else { throw ZooError.rejected(413, "通話シグナルが上限を超えました。") }
        return try JSONDecoder().decode(JSONValue.self, from: data)
    }
}

enum CallEvent: Sendable {
    case room(CallRoom)
    case joined(CallParticipant)
    case updated(CallParticipant)
    case left(String)
    case signal(String, CallSignal)

    static func decode(name: String, data: Data) throws -> Self? {
        struct Participant: Decodable { let participant: CallParticipant }
        struct Left: Decodable { let session_id: String }
        struct Signal: Decodable { let from_session_id: String; let signal: CallSignal }
        let decoder = JSONDecoder()
        switch name {
        case "room_state": return .room(try decoder.decode(CallRoom.self, from: data))
        case "participant_joined": return .joined(try decoder.decode(Participant.self, from: data).participant)
        case "participant_updated": return .updated(try decoder.decode(Participant.self, from: data).participant)
        case "participant_left": return .left(try decoder.decode(Left.self, from: data).session_id)
        case "signal":
            let value = try decoder.decode(Signal.self, from: data)
            return .signal(value.from_session_id, value.signal)
        default: return nil
        }
    }
}

struct CallInvitation: Decodable, Identifiable, Sendable {
    let id: String
    let channel_id: String
    let inviter_id: String
    let invitee_id: String
    let status: String
    let expires_at: String
}

struct CreatedCallInvitation: Decodable, Sendable {
    let id: String
    let invitee_id: String
}

// SSE の改行区切りを独立させ、分割受信や複数 data 行でも同じイベントに復元する。
struct CallEventParser {
    private var name = ""
    private var data: [String] = []
    private var byteCount = 0

    mutating func feed(_ line: String) throws -> CallEvent? {
        if line.isEmpty {
            defer { name = ""; data = []; byteCount = 0 }
            guard !data.isEmpty else { return nil }
            return try CallEvent.decode(name: name, data: Data(data.joined(separator: "\n").utf8))
        }
        byteCount += line.utf8.count
        guard byteCount <= 65_536 else { throw ZooError.invalidResponse }
        if line.hasPrefix("event:") { name = String(line.dropFirst(6)).trimmingCharacters(in: .whitespaces) }
        if line.hasPrefix("data:") {
            let value = line.dropFirst(5)
            data.append(String(value.hasPrefix(" ") ? value.dropFirst() : value))
        }
        return nil
    }
}
