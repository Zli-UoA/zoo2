import Foundation

@MainActor
protocol CallClient {
    func activeCalls() async throws -> [ActiveCallSummary]
    func observeCall(channelID: String, sessionID: String, receive: @MainActor (CallEvent) async throws -> Void) async throws
    func callCommand(channelID: String, action: String, body: [String: JSONValue]) async throws
    func observeCallInvitations(receive: @MainActor ([CallInvitation]) -> Void) async throws
    func answerCallInvitation(id: String, accept: Bool) async throws
    func isCallInvitationPending(id: String) async throws -> Bool
    func inviteToCall(channelID: String, userIDs: [String]) async throws -> [CreatedCallInvitation]
    func cancelCallInvitation(id: String) async throws
}

extension NativeChatClient {
    func activeCalls() async throws -> [ActiveCallSummary] {
        try await api.get("api/calls/active")
    }

    func isCallInvitationPending(id: String) async throws -> Bool {
        guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
        struct Response: Decodable { let pending: Bool }
        let data = try await api.data("api/call-invites/\(id)")
        return try JSONDecoder().decode(Response.self, from: data).pending
    }

    func observeCall(channelID: String, sessionID: String, receive: @MainActor (CallEvent) async throws -> Void) async throws {
        guard UUID(uuidString: channelID) != nil, UUID(uuidString: sessionID) != nil else { throw ZooError.invalidResponse }
        var request = try await api.request("api/channels/\(channelID)/call/stream", query: ["session_id": sessionID])
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        let (bytes, response) = try await api.transport.session.bytes(for: request)
        guard let response = response as? HTTPURLResponse else { throw ZooError.invalidResponse }
        try HTTPTransport.check(Data(), response)
        guard response.mimeType == "text/event-stream" else { throw ZooError.invalidResponse }
        var parser = CallEventParser()
        // AsyncBytes.lines は空行を省略するため、SSE のイベント境界をバイト列から保つ。
        var line: [UInt8] = []
        var previousWasCR = false
        for try await byte in bytes {
            try Task.checkCancellation()
            if previousWasCR && byte == 10 { previousWasCR = false; continue }
            previousWasCR = byte == 13
            if byte == 10 || byte == 13 {
                guard let text = String(bytes: line, encoding: .utf8) else { throw ZooError.invalidResponse }
                line.removeAll(keepingCapacity: true)
                if let event = try parser.feed(text) { try await receive(event) }
            } else {
                guard line.count < 65_536 else { throw ZooError.invalidResponse }
                line.append(byte)
            }
        }
        throw URLError(.networkConnectionLost)
    }

    func callCommand(channelID: String, action: String, body: [String: JSONValue]) async throws {
        guard UUID(uuidString: channelID) != nil, ["signal", "state", "heartbeat", "leave"].contains(action) else {
            throw ZooError.invalidResponse
        }
        let request = try await api.request("api/channels/\(channelID)/call/\(action)", method: "POST", body: body)
        // 通話presenceの30秒の期限内にheartbeatの再試行を終えられるようにする。
        let (data, response) = try await api.transport.send(request, timeout: .seconds(4))
        try HTTPTransport.check(data, response)
    }

    func observeCallInvitations(receive: @MainActor ([CallInvitation]) -> Void) async throws {
        try await ShapeStream(api: api).observe("api/shapes/call_invites", query: [:], as: CallInvitation.self) {
            if let values = $0 { receive(values) }
        }
    }

    func answerCallInvitation(id: String, accept: Bool) async throws {
        guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
        _ = try await api.data("api/call-invites/\(id)/\(accept ? "accept" : "decline")", method: "POST")
    }

    func inviteToCall(channelID: String, userIDs: [String]) async throws -> [CreatedCallInvitation] {
        guard UUID(uuidString: channelID) != nil, !userIDs.isEmpty, userIDs.count <= 5 else {
            throw ZooError.invalidResponse
        }
        struct Response: Decodable { let invites: [CreatedCallInvitation] }
        let data = try await api.data("api/channels/\(channelID)/call/invites", method: "POST",
                                      body: ["user_ids": .array(userIDs.map(JSONValue.string))])
        return try JSONDecoder().decode(Response.self, from: data).invites
    }

    func cancelCallInvitation(id: String) async throws {
        guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
        do { _ = try await api.data("api/call-invites/\(id)/cancel", method: "POST") }
        catch ZooError.rejected(let status, _) where status == 404 || status == 409 { }
    }
}
