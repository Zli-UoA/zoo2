import Foundation

@MainActor
protocol InboxClient: Sendable {
    var serverURL: String { get }
    func channelNoticeSnapshot() async throws -> [ChannelNotice]
    func observeInbox(receive: @MainActor ([ChannelNotice]?) -> Void) async throws
    func federatedNotices(before: String?) async throws -> FederatedNoticePage
    func federatedNotices(ids: [String]) async throws -> [FederatedNotice]
    func observeHomeUpdates(refresh: @MainActor () async -> Void) async throws
    func acknowledgeNotices(ids: [String]?) async throws
    func noticeContext(id: String) async throws -> InboxContext
    func respondToFollow(actor: String, activity: String, accept: Bool) async throws
}

extension NativeChatClient {
    func channelNoticeSnapshot() async throws -> [ChannelNotice] {
        try await ShapeStream(api: api).snapshot("api/shapes/notifications", as: ChannelNotice.self)
    }

    func observeInbox(receive: @MainActor ([ChannelNotice]?) -> Void) async throws {
        try await ShapeStream(api: api).observe("api/shapes/notifications", query: [:], as: ChannelNotice.self, receive: receive)
    }

    func federatedNotices(before: String?) async throws -> FederatedNoticePage {
        let data = try await api.data("api/activitypub/notifications", query: before.map { ["before": $0] } ?? [:])
        return try JSONDecoder().decode(FederatedNoticePage.self, from: data)
    }

    func federatedNotices(ids: [String]) async throws -> [FederatedNotice] {
        guard !ids.isEmpty else { return [] }
        let data = try await api.data("api/activitypub/notifications/entries", method: "POST",
                                      body: ["entries": .array(ids.map(JSONValue.string))])
        return try JSONDecoder().decode(FederatedNoticePage.self, from: data).items
    }

    func observeHomeUpdates(refresh: @MainActor () async -> Void) async throws {
        let server = try ConnectionSettings.secureURL(serverURL)
        guard var parts = URLComponents(url: server.appendingPathComponent("api/home/live"), resolvingAgainstBaseURL: false) else {
            throw ZooError.invalidConfiguration
        }
        parts.scheme = "wss"
        guard let url = parts.url else { throw ZooError.invalidConfiguration }
        let token = try await api.auth.accessToken()
        let socket = api.transport.session.webSocketTask(with: url)
        socket.resume()
        defer { socket.cancel(with: .goingAway, reason: nil) }
        try await withTaskCancellationHandler {
            // Web 版と同様に、認証情報は暗号化された接続の最初のフレームだけで渡す。
            let bytes = try JSONEncoder().encode(["authorization": "Bearer " + token])
            try await socket.send(.string(String(decoding: bytes, as: UTF8.self)))
            while !Task.isCancelled {
                if case .string("refresh") = try await socket.receive() { await refresh() }
            }
        } onCancel: { socket.cancel(with: .goingAway, reason: nil) }
    }

    func acknowledgeNotices(ids: [String]?) async throws {
        if let ids {
            guard !ids.isEmpty else { return }
            _ = try await api.data("api/notifications/read", method: "POST", body: ["ids": .array(ids.map(JSONValue.string))])
        } else {
            _ = try await api.data("api/notifications/read-all", method: "POST", body: [:])
        }
    }

    func noticeContext(id: String) async throws -> InboxContext {
        guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
        return try await api.get("api/notifications/\(id)/context")
    }

    func respondToFollow(actor: String, activity: String, accept: Bool) async throws {
        _ = try await api.data("api/activitypub/follows/respond", method: "POST",
                               body: ["actor": .string(actor), "activity_id": .string(activity), "accept": .bool(accept)])
    }
}
