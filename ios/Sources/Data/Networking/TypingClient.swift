import Foundation

struct TypingUser: Decodable, Equatable, Sendable, Identifiable {
    let id: String
    let display_name: String
}

@MainActor
protocol TypingClient: Sendable {
    func typingUsers(channelID: String, parentID: String?) async throws -> [TypingUser]
    func setTyping(channelID: String, parentID: String?, sessionID: UUID, active: Bool) async throws
}

extension NativeChatClient {
    func typingUsers(channelID: String, parentID: String?) async throws -> [TypingUser] {
        let data = try await typingRequest(channelID: channelID, parentID: parentID)
        return try JSONDecoder().decode([TypingUser].self, from: data)
    }

    func setTyping(channelID: String, parentID: String?, sessionID: UUID, active: Bool) async throws {
        _ = try await typingRequest(channelID: channelID, parentID: parentID, body: [
            "parent_id": parentID.map(JSONValue.string) ?? .null,
            "session_id": .string(sessionID.uuidString.lowercased()), "active": .bool(active)
        ])
    }

    private func typingRequest(channelID: String, parentID: String?, body: [String: JSONValue]? = nil) async throws -> Data {
        guard UUID(uuidString: channelID) != nil,
              parentID.map({ UUID(uuidString: $0) != nil }) ?? true else { throw ZooError.invalidResponse }
        var request = try await api.request("api/channels/\(channelID)/typing", method: body == nil ? "GET" : "POST",
                                            query: body == nil ? parentID.map { ["parent_id": $0] } ?? [:] : [:], body: body)
        // 短い有効期限の通知で、遅い要求が停止通知や次の取得を長時間妨げないようにする。
        request.timeoutInterval = 5
        let (data, response) = try await api.transport.send(request)
        try HTTPTransport.check(data, response)
        return data
    }
}
