import Foundation

struct ShapeEvent: Decodable, Sendable {
    struct Headers: Decodable, Sendable {
        let operation: String?
        let control: String?
    }
    let headers: Headers
    let key: String?
    let value: [String: JSONValue]?
}

struct ShapeState: Sendable {
    private(set) var offset = "-1"
    private(set) var handle: String?
    private(set) var cursor: String?
    private(set) var isLive = false
    private(set) var rows: [String: [String: JSONValue]] = [:]
    private var cacheBuster: String?

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
            if event.headers.control == "up-to-date" { next.isLive = true }
            guard let operation = event.headers.operation else { continue }
            guard let key = event.key else { throw ZooError.invalidResponse }
            switch operation {
            case "insert":
                guard let value = event.value else { throw ZooError.invalidResponse }
                next.rows[key] = value
            case "update":
                guard let value = event.value, var row = next.rows[key] else { throw ZooError.invalidResponse }
                row.merge(value) { _, new in new }
                next.rows[key] = row
            case "delete": next.rows[key] = nil
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

    func snapshot<T: Decodable & Sendable>(_ path: String, query: [String: String] = [:], as type: T.Type) async throws -> [T] {
        var state = ShapeState()
        while !Task.isCancelled {
            let request = try await api.request(path, query: query.merging(state.query) { _, new in new })
            let (data, response) = try await api.transport.send(request)
            try Task.checkCancellation()
            if let values = try state.apply(data, response: response, as: type) { return values }
            if state.offset == "-1" { try await Task.sleep(for: .milliseconds(250)) }
        }
        throw CancellationError()
    }

    func observe<T: Decodable & Sendable>(_ path: String, query: [String: String], as type: T.Type,
                                          receive: @MainActor ([T]?) -> Void) async throws {
        var state = ShapeState()
        while !Task.isCancelled {
            let parameters = query.merging(state.query) { _, new in new }
            let request = try await api.request(path, query: parameters)
            let (data, response) = try await api.transport.send(request)
            try Task.checkCancellation()
            let snapshot = try state.apply(data, response: response, as: type)
            if let snapshot { receive(snapshot) }
            else if state.offset == "-1" { receive(nil) }
            // 即時の空応答や継続的な shape 交換でも要求を連打しない。
            if state.isLive || state.offset == "-1" { try await Task.sleep(for: .milliseconds(250)) }
        }
        throw CancellationError()
    }
}
