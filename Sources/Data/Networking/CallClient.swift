import Foundation

@MainActor
protocol CallClient {
    func observeCall(channelID: String, sessionID: String, receive: @MainActor (CallEvent) async throws -> Void) async throws
    func callCommand(channelID: String, action: String, body: [String: JSONValue]) async throws
    func observeCallInvitations(receive: @MainActor ([CallInvitation]) -> Void) async throws
    func answerCallInvitation(id: String, accept: Bool) async throws
}

extension NativeChatClient {
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
        for try await byte in bytes {
            try Task.checkCancellation()
            if byte == 10 {
                if line.last == 13 { line.removeLast() }
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
        _ = try await api.data("api/channels/\(channelID)/call/\(action)", method: "POST", body: body)
    }

    func observeCallInvitations(receive: @MainActor ([CallInvitation]) -> Void) async throws {
        try await ShapeStream(api: api).observe("api/shapes/call_invites", query: [:], as: CallInvitation.self) { receive($0 ?? []) }
    }

    func answerCallInvitation(id: String, accept: Bool) async throws {
        guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
        _ = try await api.data("api/call-invites/\(id)/\(accept ? "accept" : "decline")", method: "POST")
    }
}
