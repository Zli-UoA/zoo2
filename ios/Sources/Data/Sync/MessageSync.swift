import Foundation

struct MessageFeedRequest: Equatable, Sendable {
    var parentID: String? = nil
    var limit = 50
    var focusID: String? = nil
    var key: String { parentID ?? "root" }
}

struct MessageFeedSnapshot: Sendable {
    let messages: [Message]?
    var fromCache = false
    var hasMore = false
    var loadedLimit = 0
}

struct MessageWindow: Codable, Sendable {
    var parentID: String?
    var limit = 0
    var hasMore = true
    var oldestDate: String?
    var oldestID: String?
}

struct SnapshotBoundary: Codable, Sendable {
    let xmin: String
    let xmax: String
    let xip_list: [String]
    let database_lsn: String
    var keys: Set<String> = []

    enum CodingKeys: String, CodingKey { case xmin, xmax, xip_list, database_lsn, keys }

    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        xmin = try values.decode(String.self, forKey: .xmin)
        xmax = try values.decode(String.self, forKey: .xmax)
        xip_list = try values.decode([String].self, forKey: .xip_list)
        database_lsn = try values.decode(String.self, forKey: .database_lsn)
        keys = try values.decodeIfPresent(Set<String>.self, forKey: .keys) ?? []
    }

    func contains(_ txids: [UInt64]) throws -> Bool {
        guard let min = UInt64(xmin), let max = UInt64(xmax) else { throw ZooError.invalidResponse }
        let active = try Set(xip_list.map { value -> UInt64 in
            guard let id = UInt64(value) else { throw ZooError.invalidResponse }
            return id
        })
        // Electricの32bit xidを、スナップショットのxmaxに最も近いepochへ戻す。
        let latest = txids.map { max &+ UInt64(bitPattern: Int64(Int32(bitPattern: UInt32(truncatingIfNeeded: $0 &- max)))) }.max()
        guard let latest else { return false }
        return latest < min || (latest < max && !active.contains(latest))
    }
}

struct MessageSyncState: Codable, Sendable {
    var shape = ShapeState(offset: "now")
    var windows: [String: MessageWindow] = [:]
    var boundaries: [SnapshotBoundary] = []

    var query: [String: String] {
        shape.query.merging(["log": "changes_only", "replica": "full"]) { _, value in value }
    }

    mutating func apply(_ data: Data, response: HTTPURLResponse) throws {
        if response.statusCode == 409 {
            self = MessageSyncState()
            shape.reset(handle: response.value(forHTTPHeaderField: "electric-handle"))
            shape.offset = "now"
            return
        }
        try HTTPTransport.check(data, response)
        shape.isLive = false
        let events = data.isEmpty ? [] : try JSONDecoder().decode([ShapeEvent].self, from: data)
        if events.contains(where: { $0.headers.control == "must-refetch" }) {
            self = MessageSyncState()
            return
        }
        shape.handle = response.value(forHTTPHeaderField: "electric-handle") ?? shape.handle
        shape.offset = response.value(forHTTPHeaderField: "electric-offset") ?? shape.offset
        shape.cursor = response.value(forHTTPHeaderField: "electric-cursor") ?? shape.cursor
        guard shape.handle != nil, shape.offset != "now" else { throw ZooError.invalidResponse }
        for event in events {
            try shape.applyMove(event.headers)
            if let lsn = event.headers.lsn.flatMap(UInt64.init) {
                boundaries.removeAll { (UInt64($0.database_lsn) ?? .max) <= lsn }
            }
            if let operation = event.headers.operation {
                guard let key = event.key else { throw ZooError.invalidResponse }
                let duplicate = try boundaries.contains {
                    guard $0.keys.contains(key) else { return false }
                    return try $0.contains(event.headers.txids ?? [])
                }
                if duplicate { continue }
                switch operation {
                case "delete": shape.removeRow(key)
                case "insert", "update":
                    guard let value = event.value else { throw ZooError.invalidResponse }
                    let row = (shape.rows[key] ?? [:]).merging(value) { _, value in value }
                    // replica=fullでも未取得の古い行は増やさず、表示している範囲だけを保持する。
                    if shape.rows[key] != nil || includes(row) {
                        shape.rows[key] = row
                        shape.recordTags(event.headers, key: key)
                    }
                default: throw ZooError.invalidResponse
                }
            }
            if event.headers.control == "up-to-date" {
                shape.isLive = true
                if let lsn = event.headers.global_last_seen_lsn.flatMap(UInt64.init) {
                    boundaries.removeAll { (UInt64($0.database_lsn) ?? .max) <= lsn }
                }
            }
        }
        if response.statusCode == 204 || response.value(forHTTPHeaderField: "electric-up-to-date") != nil { shape.isLive = true }
    }

    private func includes(_ row: [String: JSONValue]) -> Bool {
        let parent = row["parent_id"]?.text
        guard let window = windows[parent ?? "root"] else { return false }
        guard let date = row["created_at"]?.text, let id = row["id"]?.text else { return false }
        guard let oldest = window.oldestDate else { return !window.hasMore }
        return date > oldest || (date == oldest && id >= (window.oldestID ?? ""))
    }

    mutating func addSnapshot(_ data: Data, response: HTTPURLResponse) throws -> [[String: JSONValue]] {
        struct Subset: Decodable { let data: [ShapeEvent]; var metadata: SnapshotBoundary }
        try HTTPTransport.check(data, response)
        guard response.value(forHTTPHeaderField: "electric-handle") == shape.handle else { throw MessageSyncError.reset }
        var subset = try JSONDecoder().decode(Subset.self, from: data)
        guard UInt64(subset.metadata.database_lsn) != nil, UInt64(subset.metadata.xmin) != nil,
              UInt64(subset.metadata.xmax) != nil else { throw ZooError.invalidResponse }
        var values: [[String: JSONValue]] = []
        for event in subset.data {
            guard let key = event.key, let value = event.value, event.headers.operation == "insert" else { throw ZooError.invalidResponse }
            shape.rows[key] = value
            shape.recordTags(event.headers, key: key)
            subset.metadata.keys.insert(key)
            values.append(value)
        }
        boundaries.append(subset.metadata)
        return values
    }
}

enum MessageSyncError: Error { case reset }

extension JSONValue {
    var text: String? { if case .string(let value) = self { value } else { nil } }
}

@MainActor
struct MessageSync {
    let api: APIClient
    let sync: SyncSession
    let channelID: String
    var key: String { "messages-v1:" + channelID }

    func observe(_ request: MessageFeedRequest, receive: @MainActor (MessageFeedSnapshot) -> Void) async throws {
        let version = sync.channelVersion(channelID)
        var state = try await sync.load(key, as: MessageSyncState.self) ?? MessageSyncState()
        if state.shape.isLive {
            let messages = try await decodeShapeRows(state.shape.rows, as: Message.self)
            try sync.check(channelID: channelID, channelVersion: version)
            receive(MessageFeedSnapshot(messages: messages, fromCache: true,
                                        hasMore: state.windows[request.key]?.hasMore ?? true,
                                        loadedLimit: state.windows[request.key]?.limit ?? 0))
        }
        state.shape.isLive = false
        var sseFailures = 0
        var catchingUp = true
        do {
            while !Task.isCancelled {
                try sync.check(channelID: channelID)
                if state.shape.handle == nil || state.shape.offset == "now" {
                    let request = try await api.request("api/shapes/messages", query: state.query.merging(["channel_id": channelID]) { _, new in new })
                    let (data, response) = try await api.transport.send(request)
                    state = try await updateMessageState(state, data: data, response: response)
                    if state.shape.handle == nil || state.shape.offset == "now" {
                        try await Task.sleep(for: .milliseconds(500))
                        continue
                    }
                }
                do {
                    try await loadWindow(request, state: &state)
                    if let focusID = request.focusID { try await loadMessage(focusID, state: &state) }
                    if let parentID = request.parentID { try await loadMessage(parentID, state: &state) }
                } catch MessageSyncError.reset {
                    state = MessageSyncState()
                    try await sync.store(state, key: key, channelID: channelID, channelVersion: version)
                    receive(MessageFeedSnapshot(messages: nil))
                    try await Task.sleep(for: .milliseconds(500))
                    continue
                }
                if state.shape.isLive { try await sync.store(state, key: key, channelID: channelID, channelVersion: version) }
                var query = state.query
                query["channel_id"] = channelID
                if catchingUp { query["live"] = nil; query["cursor"] = nil }
                let useSSE = !catchingUp && state.shape.isLive && sseFailures < 3
                let started = ContinuousClock.now
                do {
                    try await ElectricTransport(api: api).read("api/shapes/messages", query: query, sse: useSSE) { data, response in
                        let next = try await updateMessageState(state, data: data, response: response)
                        let messages = next.shape.isLive ? try await decodeShapeRows(next.shape.rows, as: Message.self) : nil
                        if next.shape.isLive || next.shape.offset == "now" {
                            try await sync.store(next, key: key, channelID: channelID, channelVersion: version)
                        }
                        state = next
                        if messages != nil || state.shape.offset == "now" {
                            receive(MessageFeedSnapshot(messages: messages, hasMore: state.windows[request.key]?.hasMore ?? true,
                                                        loadedLimit: state.windows[request.key]?.limit ?? 0))
                        }
                    }
                } catch where useSSE && isSSETransportFailure(error) && !Task.isCancelled {
                    sseFailures += 1
                    try await Task.sleep(for: .seconds(1))
                    continue
                }
                catchingUp = !state.shape.isLive
                if useSSE { sseFailures = started.duration(to: .now) < .seconds(1) ? sseFailures + 1 : 0 }
                try await Task.sleep(for: .milliseconds(250))
            }
        } catch {
            try sync.check(channelID: channelID, channelVersion: version)
            if case ZooError.rejected(let status, _) = error, status == 403 || status == 404 {
                try await sync.revokeChannel(channelID)
            }
            throw error
        }
        throw CancellationError()
    }

    private func loadWindow(_ request: MessageFeedRequest, state: inout MessageSyncState) async throws {
        var window = state.windows[request.key] ?? MessageWindow(parentID: request.parentID)
        while window.limit < request.limit && window.hasMore {
            var parameters: [String: String] = [:]
            var predicates = ["deleted_at IS NULL"]
            if let parent = request.parentID {
                guard UUID(uuidString: parent) != nil else { throw ZooError.invalidResponse }
                parameters["1"] = parent
                predicates.append("parent_id = $1")
            } else { predicates.append("parent_id IS NULL") }
            if let date = window.oldestDate, let id = window.oldestID {
                let dateIndex = parameters.count + 1
                let idIndex = dateIndex + 1
                parameters[String(dateIndex)] = date
                parameters[String(idIndex)] = id
                predicates.append("(created_at < $\(dateIndex) OR (created_at = $\(dateIndex) AND id < $\(idIndex)))")
            }
            let values = try await subset(where: predicates.joined(separator: " AND "), parameters: parameters, limit: 50, state: &state)
            let sorted = values.sorted {
                let left = $0["created_at"]?.text ?? "", right = $1["created_at"]?.text ?? ""
                return left == right ? ($0["id"]?.text ?? "") > ($1["id"]?.text ?? "") : left > right
            }
            if let last = sorted.last {
                window.oldestDate = last["created_at"]?.text
                window.oldestID = last["id"]?.text
                guard window.oldestDate != nil, window.oldestID != nil else { throw ZooError.invalidResponse }
            }
            window.limit += 50
            window.hasMore = values.count == 50
            state.windows[request.key] = window
        }
    }

    private func loadMessage(_ id: String, state: inout MessageSyncState) async throws {
        guard UUID(uuidString: id) != nil else { throw ZooError.invalidResponse }
        if state.shape.rows.values.contains(where: { $0["id"]?.text == id }) { return }
        let rows = try await subset(where: "id = $1", parameters: ["1": id], limit: 1, state: &state)
        if let parent = rows.first?["parent_id"]?.text, parent != id,
           !state.shape.rows.values.contains(where: { $0["id"]?.text == parent }) {
            _ = try await subset(where: "id = $1", parameters: ["1": parent], limit: 1, state: &state)
        }
    }

    private func subset(where predicate: String, parameters: [String: String], limit: Int,
                        state: inout MessageSyncState) async throws -> [[String: JSONValue]] {
        var query = state.query
        query["live"] = nil; query["cursor"] = nil
        query["channel_id"] = channelID
        query["subset__where"] = predicate
        query["subset__params"] = String(decoding: try JSONEncoder().encode(parameters), as: UTF8.self)
        query["subset__order_by"] = "created_at DESC, id DESC"
        query["subset__limit"] = String(limit)
        let request = try await api.request("api/shapes/messages", query: query)
        let (data, response) = try await api.transport.send(request)
        try sync.check(channelID: channelID)
        if response.statusCode == 409 { throw MessageSyncError.reset }
        return try state.addSnapshot(data, response: response)
    }
}

@concurrent private func updateMessageState(_ original: MessageSyncState, data: Data,
                                           response: HTTPURLResponse) async throws -> MessageSyncState {
    var state = original
    try state.apply(data, response: response)
    return state
}
