import Foundation

struct ShapeEntry: Decodable, Sendable {
    struct Headers: Decodable, Sendable {
        let operation: String?
        let control: String?
    }
    let key: String?
    let value: [String: JSONValue]?
    let headers: Headers
}

struct ShapeState: Sendable {
    var records: [String: [String: JSONValue]] = [:]
    var offset = "-1"
    var handle: String?
    var cursor: String?
    var ready = false

    mutating func apply(_ entries: [ShapeEntry]) throws {
        for entry in entries {
            if entry.headers.control == "must-refetch" { self = ShapeState(); return }
            if entry.headers.control == "up-to-date" { ready = true }
            guard let operation = entry.headers.operation else { continue }
            guard let key = entry.key else { throw ZooError.invalidResponse }
            switch operation {
            case "delete": records.removeValue(forKey: key)
            case "insert", "update":
                guard let value = entry.value else { throw ZooError.invalidResponse }
                var normalized = value
                for field in ["is_private", "is_dm", "is_bot", "is_read"] {
                    if case .string(let text) = normalized[field] {
                        if ["true", "t"].contains(text) { normalized[field] = .bool(true) }
                        if ["false", "f"].contains(text) { normalized[field] = .bool(false) }
                    }
                }
                if operation == "insert" { records[key] = normalized }
                else { records[key, default: [:]].merge(normalized) { _, new in new } }
            default: throw ZooError.invalidResponse
            }
        }
    }

    func decode<T: Decodable>(_ type: T.Type) throws -> [T] {
        try records.values.map { try JSONDecoder().decode(type, from: JSONEncoder().encode($0)) }
    }
}

@MainActor
final class ShapeStream {
    private var task: Task<Void, Never>?

    func stop() { task?.cancel(); task = nil }

    func start(api: APIClient, shape: String, channelID: String? = nil,
               update: @escaping @MainActor (ShapeState) throws -> Void,
               failure: @escaping @MainActor (Error) -> Void,
               recovered: @escaping @MainActor () -> Void) {
        stop()
        task = Task {
            var state = ShapeState()
            var backoff = 1
            while !Task.isCancelled {
                do {
                    var query = ["offset": state.offset]
                    if let channelID { query["channel_id"] = channelID }
                    if let handle = state.handle { query["handle"] = handle }
                    if let cursor = state.cursor { query["cursor"] = cursor }
                    if state.ready { query["live"] = "true" }
                    let request = try await api.request("api/shapes/\(shape)", query: query)
                    let (data, response) = try await api.transport.send(request)
                    try Task.checkCancellation()
                    if response.statusCode == 409 {
                        state = ShapeState()
                        try update(state)
                        try await Task.sleep(for: .seconds(backoff))
                        backoff = min(backoff * 2, 30)
                        continue
                    }
                    try HTTPTransport.check(data, response)
                    if response.statusCode != 204 {
                        let entries = try JSONDecoder().decode([ShapeEntry].self, from: data)
                        if entries.contains(where: { $0.headers.control == "must-refetch" }) {
                            state = ShapeState()
                            try update(state)
                            try await Task.sleep(for: .seconds(1))
                            continue
                        }
                        guard let offset = response.value(forHTTPHeaderField: "electric-offset"),
                              let handle = response.value(forHTTPHeaderField: "electric-handle") else {
                            throw ZooError.invalidResponse
                        }
                        // Publish only complete snapshots; partial pages remain internal.
                        var next = state
                        try next.apply(entries)
                        next.offset = offset
                        next.handle = handle
                        next.cursor = response.value(forHTTPHeaderField: "electric-cursor")
                        if next.ready { try update(next) }
                        state = next
                    }
                    recovered()
                    backoff = 1
                    try await Task.sleep(for: .milliseconds(200))
                } catch {
                    if Task.isCancelled { return }
                    if case ZooError.rejected(let code, _) = error, code == 403 || code == 404 {
                        try? update(ShapeState())
                        failure(error)
                        return
                    }
                    failure(error)
                    if case ZooError.expiredSession = error { return }
                    do { try await Task.sleep(for: .seconds(backoff)) } catch { return }
                    backoff = min(backoff * 2, 30)
                }
            }
        }
    }
}
