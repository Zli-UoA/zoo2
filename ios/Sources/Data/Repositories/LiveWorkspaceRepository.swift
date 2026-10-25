import Foundation

@MainActor
final class LiveWorkspaceRepository: WorkspaceRepository {
    let state = WorkspaceState()
    var onChannelsChanged: (() -> Void)?
    var onSessionExpired: (() -> Void)?
    private let session: RepositorySession
    private var streams: [ShapeStream] = []
    private var readTask: Task<Void, Never>?
    private var generation = UUID()

    init(session: RepositorySession) { self.session = session }

    func start() {
        stop()
        state.channelPhase = state.channels.isEmpty ? .loading : .resyncing
        streams = [
            subscribe("channels", type: Channel.self) { [weak self] rows, ready in
                guard let self else { return }
                self.state.channels = rows
                self.state.channelPhase = ready ? .ready : .resyncing
                self.onChannelsChanged?()
            },
            subscribe("users", type: Person.self) { [weak self] rows, _ in self?.state.people = rows },
            subscribe("channel_users", type: Membership.self) { [weak self] rows, _ in self?.state.memberships = rows },
            subscribe("notifications", type: Activity.self) { [weak self] rows, _ in
                self?.state.activities = rows.sorted { $0.created_at > $1.created_at }
            }
        ]
        let epoch = generation
        readTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, epoch == self.generation else { return }
                await self.refreshReadStates(epoch: epoch)
                do { try await Task.sleep(for: .seconds(15)) } catch { return }
            }
        }
    }

    func stop() {
        generation = UUID()
        streams.forEach { $0.stop() }
        streams = []
        readTask?.cancel()
        readTask = nil
    }

    func reset() { stop(); state.reset() }

    private func subscribe<T: Decodable>(_ shape: String, type: T.Type,
                                         receive: @escaping @MainActor ([T], Bool) -> Void) -> ShapeStream {
        let stream = ShapeStream()
        let epoch = generation
        var snapshotReady = false
        stream.start(api: session.api, shape: shape, update: { [weak self] snapshot in
            guard let self, epoch == self.generation else { return }
            try self.session.check()
            receive(try snapshot.decode(type), snapshot.ready)
            snapshotReady = snapshot.ready
        }, failure: { [weak self] error in
            guard let self, epoch == self.generation else { return }
            if case ZooError.expiredSession = error { self.onSessionExpired?(); return }
            self.state.syncErrors[shape] = error.localizedDescription
            if shape == "channels" {
                if case ZooError.rejected(let code, _) = error, code == 403 || code == 404 {
                    self.state.channelPhase = .accessDenied
                } else { self.state.channelPhase = .failed(error.localizedDescription) }
                self.onChannelsChanged?()
            }
        }, recovered: { [weak self] in
            guard let self, epoch == self.generation, snapshotReady else { return }
            self.state.syncErrors.removeValue(forKey: shape)
            if shape == "channels", self.state.channelPhase != .ready {
                self.state.channelPhase = .ready
                self.onChannelsChanged?()
            }
        })
        return stream
    }

    private func refreshReadStates(epoch: UUID) async {
        do {
            let states: [ReadState] = try await session.api.get("api/channel-read-states")
            try session.check()
            guard epoch == generation else { return }
            state.unread = Set(states.filter(\.has_unread).map(\.channel_id))
            state.syncErrors.removeValue(forKey: "read-state")
        } catch {
            guard epoch == generation, !Task.isCancelled else { return }
            if case ZooError.expiredSession = error { onSessionExpired?() }
            else { state.syncErrors["read-state"] = error.localizedDescription }
        }
    }

    func createChannel(name: String, description: String, isPrivate: Bool) async throws -> Channel {
        try await session.write("api/channels", body: ["channel_name": .string(name), "description": .string(description), "is_private": .bool(isPrivate)])
    }

    func startDM(_ person: Person) async throws -> Channel {
        try await session.write("api/dms", body: ["target_user_ids": .array([.string(person.id)])])
    }

    func join(_ channel: Channel) async throws {
        let _: Membership = try await session.write("api/channels/\(channel.id)/join")
    }

    func leave(_ channel: Channel) async throws {
        let _: JSONValue = try await session.write("api/channels/\(channel.id)/leave")
    }

    func invite(_ person: Person, to channel: Channel) async throws {
        let _: Membership = try await session.write("api/channels/\(channel.id)/users", body: ["user_id": .string(person.id)])
    }

    func updateChannel(_ channel: Channel, name: String, description: String) async throws {
        let _: Channel = try await session.write("api/channels/\(channel.id)", method: "PATCH", body: ["channel_name": .string(name), "description": .string(description)])
    }

    func setArchived(_ channel: Channel, archived: Bool) async throws {
        let _: Channel = try await session.write("api/channels/\(channel.id)/\(archived ? "archive" : "unarchive")")
    }

    func markRead(_ channelID: String, messageID: String) async throws {
        let _: JSONValue = try await session.write("api/channels/\(channelID)/read-state", body: ["message_id": .string(messageID)])
        state.unread.remove(channelID)
    }

    func readActivity(_ activity: Activity) async throws {
        let _: Activity = try await session.write("api/notifications/\(activity.id)/read", method: "PATCH")
    }

    func updateName(_ name: String) async throws {
        var person: Person = try await session.write("api/users/me", method: "PATCH", body: ["display_name": .string(name)])
        person.is_admin = state.me?.is_admin
        state.me = person
    }
}
