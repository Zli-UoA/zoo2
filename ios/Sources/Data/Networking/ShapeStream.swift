import Foundation

struct ShapeEvent: Codable, Sendable {
    struct Headers: Codable, Sendable {
        let operation: String?
        let control: String?
        var txids: [UInt64]? = nil
        var global_last_seen_lsn: String? = nil
        var lsn: String? = nil
        var event: String? = nil
        var tags: [String]? = nil
        var removed_tags: [String]? = nil
        var active_conditions: [Bool]? = nil
        var patterns: [ShapeMovePattern]? = nil
    }
    let headers: Headers
    let key: String?
    let value: [String: JSONValue]?
}

struct ShapeMovePattern: Codable, Sendable {
    let pos: Int
    let value: String
}

struct ShapeRowVisibility: Codable, Sendable, Equatable {
    var tags: Set<String> = []
    var conditions: [Bool]?

    private func parts(_ tag: String) -> [Substring] { tag.split(separator: "/", omittingEmptySubsequences: false) }

    func matches(_ tag: String, _ pattern: ShapeMovePattern) -> Bool {
        let values = parts(tag)
        return values.indices.contains(pattern.pos) && !values[pattern.pos].isEmpty && values[pattern.pos] == pattern.value
    }

    var visible: Bool {
        guard let conditions else { return !tags.isEmpty }
        return tags.contains { tag in
            parts(tag).enumerated().allSatisfy { index, value in
                value.isEmpty || (conditions.indices.contains(index) && conditions[index])
            }
        }
    }
}

struct ShapeState: Codable, Sendable, Equatable {
    var offset = "-1"
    var handle: String?
    var cursor: String?
    var isLive = false
    var rows: [String: [String: JSONValue]] = [:]
    var visibility: [String: ShapeRowVisibility] = [:]
    private var cacheBuster: String?

    init(offset: String = "-1") { self.offset = offset }

    var query: [String: String] {
        var result = ["offset": offset]
        result["handle"] = handle
        result["cache-buster"] = cacheBuster
        if isLive {
            result["live"] = "true"
            result["cursor"] = cursor
        }
        return result
    }

    mutating func reset(handle: String? = nil) {
        self = ShapeState()
        self.handle = handle
        cacheBuster = UUID().uuidString
    }

    mutating func recordTags(_ headers: ShapeEvent.Headers, key: String) {
        guard headers.tags != nil || headers.removed_tags != nil || headers.active_conditions != nil else { return }
        var row = visibility[key] ?? ShapeRowVisibility()
        row.tags.formUnion(headers.tags ?? [])
        row.tags.subtract(headers.removed_tags ?? [])
        if let conditions = headers.active_conditions, !conditions.isEmpty { row.conditions = conditions }
        visibility[key] = row
    }

    mutating func removeRow(_ key: String) {
        rows[key] = nil
        visibility[key] = nil
    }

    mutating func applyMove(_ headers: ShapeEvent.Headers) throws {
        guard let event = headers.event else { return }
        guard ["move-out", "move-in"].contains(event), let patterns = headers.patterns,
              patterns.allSatisfy({ $0.pos >= 0 }) else { throw ZooError.invalidResponse }
        for pattern in patterns {
            for (key, original) in visibility where original.tags.contains(where: { original.matches($0, pattern) }) {
                var row = original
                if var conditions = row.conditions {
                    guard conditions.indices.contains(pattern.pos) else { throw ZooError.invalidResponse }
                    conditions[pattern.pos] = event == "move-in"
                    row.conditions = conditions
                } else if event == "move-out" {
                    row.tags = row.tags.filter { !row.matches($0, pattern) }
                }
                if event == "move-out" && !row.visible { removeRow(key) }
                else { visibility[key] = row }
            }
        }
    }

    // デコードが完了するまでカーソルを進めず、再送された部分更新も同じ行に適用する。
    mutating func apply<T: Decodable & Sendable>(_ data: Data, response: HTTPURLResponse,
                                                as type: T.Type) throws -> [T]? {
        if response.statusCode == 409 {
            reset(handle: response.value(forHTTPHeaderField: "electric-handle"))
            return nil
        }
        try HTTPTransport.check(data, response)
        let events = data.isEmpty ? [] : try JSONDecoder().decode([ShapeEvent].self, from: data)
        if events.contains(where: { $0.headers.control == "must-refetch" }) {
            reset(handle: response.value(forHTTPHeaderField: "electric-handle"))
            return nil
        }
        var next = self
        next.handle = response.value(forHTTPHeaderField: "electric-handle") ?? handle
        next.offset = response.value(forHTTPHeaderField: "electric-offset") ?? offset
        next.cursor = response.value(forHTTPHeaderField: "electric-cursor") ?? cursor
        guard next.handle != nil, next.offset != "-1" else { throw ZooError.invalidResponse }
        next.cacheBuster = nil
        if response.statusCode == 204 || response.value(forHTTPHeaderField: "electric-up-to-date") != nil {
            next.isLive = true
        }
        for event in events {
            try next.applyMove(event.headers)
            if event.headers.control == "up-to-date" { next.isLive = true }
            guard let operation = event.headers.operation else { continue }
            guard let key = event.key else { throw ZooError.invalidResponse }
            switch operation {
            case "insert":
                guard let value = event.value else { throw ZooError.invalidResponse }
                next.rows[key] = value
                next.recordTags(event.headers, key: key)
            case "update":
                guard let value = event.value, var row = next.rows[key] else { throw ZooError.invalidResponse }
                row.merge(value) { _, new in new }
                next.rows[key] = row
                next.recordTags(event.headers, key: key)
                if next.visibility[key]?.visible == false { next.removeRow(key) }
            case "delete": next.removeRow(key)
            default: throw ZooError.invalidResponse
            }
        }
        let values: [T]? = next.isLive
            ? try JSONDecoder().decode([T].self, from: JSONEncoder().encode(Array(next.rows.values))) : nil
        self = next
        return values
    }
}

@MainActor
struct ShapeStream {
    let api: APIClient
    var sync: SyncSession? = nil

    private func cacheKey(_ path: String, query: [String: String], prefix: String) -> String {
        prefix + path + query.sorted { $0.key < $1.key }.map { "|\($0.key)=\($0.value)" }.joined()
    }

    func snapshot<T: Decodable & Sendable>(_ path: String, query: [String: String] = [:], as type: T.Type,
                                          restoreCached: Bool = true) async throws -> [T] {
        let key = cacheKey(path, query: query, prefix: "snapshot:")
        let version = query["channel_id"].flatMap { sync?.channelVersion($0) }
        let revocationVersion = sync?.revocationVersion(for: path)
        var state = try await sync?.load(key, as: ShapeState.self) ?? ShapeState()
        var cached: [T]? = restoreCached && state.isLive ? try await decodeShapeRows(state.rows, as: type) : nil
        try sync?.check(channelID: query["channel_id"], channelVersion: version)
        try sync?.checkRevocationVersion(revocationVersion)
        state.isLive = false
        do {
            while !Task.isCancelled {
                let request = try await api.request(path, query: query.merging(state.query) { _, new in new })
                let (data, response) = try await api.transport.send(request)
                try Task.checkCancellation()
                let result = try await applyShape(state, data: data, response: response, as: type)
                state = result.state
                if state.offset == "-1" { cached = nil }
                if state.isLive || state.offset == "-1" {
                    try await sync?.storeRecovery(state, key: key, channelID: query["channel_id"], channelVersion: version,
                                          revocationVersion: revocationVersion)
                }
                try sync?.checkRevocationVersion(revocationVersion)
                if let values = result.values { return values }
                if state.offset == "-1" { try await Task.sleep(for: .milliseconds(250)) }
            }
            throw CancellationError()
        } catch {
            try sync?.check(channelID: query["channel_id"], channelVersion: version)
            try sync?.checkRevocationVersion(revocationVersion)
            if isOfflineFailure(error), let cached { return cached }
            if case ZooError.rejected(let status, _) = error, status == 403 || status == 404 {
                if let channelID = query["channel_id"] { try await sync?.revokeChannel(channelID) }
                else if let sync { try await sync.cache.remove(key, lease: sync.lease) }
            }
            throw error
        }
    }

    func observe<T: Decodable & Sendable>(_ path: String, query: [String: String], as type: T.Type,
                                          restoreCached: Bool = true, receive: @MainActor ([T]?) -> Void) async throws {
        let key = cacheKey(path, query: query, prefix: "shape:")
        let version = query["channel_id"].flatMap { sync?.channelVersion($0) }
        let revocationVersion = sync?.revocationVersion(for: path)
        var state = try await sync?.load(key, as: ShapeState.self) ?? ShapeState()
        if state.isLive && restoreCached {
            let rows = try await decodeShapeRows(state.rows, as: type)
            try sync?.check(channelID: query["channel_id"], channelVersion: version)
            try sync?.checkRevocationVersion(revocationVersion)
            receive(rows)
        }
        var sseFailures = 0
        do {
            while !Task.isCancelled {
                try sync?.check(channelID: query["channel_id"])
                let parameters = query.merging(state.query) { _, new in new }
                let useSSE = sync != nil && state.isLive && sseFailures < 3
                let started = ContinuousClock.now
                do {
                    let transportResult = try await ElectricTransport(api: api).read(path, query: parameters, sse: useSSE) { data, response in
                        var pendingState = state
                        if sync != nil { pendingState.isLive = false }
                        let result = try await applyShape(pendingState, data: data, response: response, as: type)
                        if result.state.isLive || result.state.offset == "-1" {
                            try await sync?.storeRecovery(result.state, key: key, channelID: query["channel_id"], channelVersion: version,
                                                  revocationVersion: revocationVersion)
                        }
                        try sync?.checkRevocationVersion(revocationVersion)
                        state = result.state
                        if let snapshot = result.values { receive(snapshot) }
                        else if state.offset == "-1" { receive(nil) }
                    }
                    if case .longPolling = transportResult { sseFailures = 3 }
                } catch where useSSE && isSSETransportFailure(error) && !Task.isCancelled {
                    sseFailures += 1
                    try await Task.sleep(for: .seconds(1))
                    continue
                }
                if useSSE && sseFailures < 3 { sseFailures = started.duration(to: .now) < .seconds(1) ? sseFailures + 1 : 0 }
                // 即時の空応答や継続的な shape 交換でも要求を連打しない。
                if state.isLive || state.offset == "-1" { try await Task.sleep(for: .milliseconds(250)) }
            }
            throw CancellationError()
        } catch {
            try sync?.check(channelID: query["channel_id"], channelVersion: version)
            try sync?.checkRevocationVersion(revocationVersion)
            if case ZooError.rejected(let status, _) = error, status == 403 || status == 404 {
                if let channelID = query["channel_id"] { try await sync?.revokeChannel(channelID) }
                else if let sync { try await sync.cache.remove(key, lease: sync.lease) }
            }
            throw error
        }
    }
}

@concurrent func decodeShapeRows<T: Decodable & Sendable>(_ rows: [String: [String: JSONValue]], as type: T.Type) async throws -> [T] {
    try JSONDecoder().decode([T].self, from: JSONEncoder().encode(Array(rows.values)))
}

@concurrent func applyShape<T: Decodable & Sendable>(_ original: ShapeState, data: Data,
                                                    response: HTTPURLResponse, as type: T.Type) async throws -> (state: ShapeState, values: [T]?) {
    var state = original
    let values = try state.apply(data, response: response, as: type)
    return (state, values)
}
