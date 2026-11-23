import Foundation

enum ElectricReadResult: Sendable { case complete, longPolling }

func isSSETransportFailure(_ error: Error) -> Bool {
    if let error = error as? URLError { return error.code != .cancelled }
    if case ZooError.invalidResponse = error { return true }
    // 認証切れ・権限喪失は即座に伝え、SSEだけを拒否するプロキシでは別の転送方式を試す。
    if case ZooError.rejected(let status, _) = error { return ![401, 403, 404].contains(status) }
    return error is DecodingError
}

struct ElectricSSEParser: Sendable {
    private var line: [UInt8] = []
    private var dataLines: [String] = []
    private var carriageReturn = false
    private var size = 0
    var hasPendingEvent: Bool { !line.isEmpty || !dataLines.isEmpty }

    mutating func feed(_ byte: UInt8) throws -> Data? {
        if byte == 10 && carriageReturn { carriageReturn = false; return nil }
        carriageReturn = byte == 13
        if byte != 10 && byte != 13 {
            line.append(byte)
            size += 1
            guard size <= 16 * 1024 * 1024 else { throw ZooError.invalidResponse }
            return nil
        }
        defer { line.removeAll(keepingCapacity: true) }
        if line.isEmpty {
            defer { dataLines.removeAll(keepingCapacity: true); size = 0 }
            return dataLines.isEmpty ? nil : Data(dataLines.joined(separator: "\n").utf8)
        }
        guard let text = String(bytes: line, encoding: .utf8) else { throw ZooError.invalidResponse }
        if text.hasPrefix("data:") {
            let value = text.dropFirst(5)
            dataLines.append(String(value.first == " " ? value.dropFirst() : value))
        }
        return nil
    }
}

@MainActor
struct ElectricTransport {
    let api: APIClient

    func read(_ path: String, query: [String: String], sse: Bool,
              receive: @MainActor (Data, HTTPURLResponse) async throws -> Void) async throws -> ElectricReadResult {
        var query = query
        if sse { query["live_sse"] = "true" }
        var request = try await api.request(path, query: query)
        if sse {
            request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
            return try await api.transport.consumeElectricSSE(request, receive: receive)
        } else {
            let (data, response) = try await api.transport.send(request)
            try Task.checkCancellation()
            try await receive(data, response)
            return .complete
        }
    }
}

extension HTTPTransport {
    @concurrent func consumeElectricSSE(_ request: URLRequest,
                                        receive: @MainActor @Sendable (Data, HTTPURLResponse) async throws -> Void) async throws -> ElectricReadResult {
        let (bytes, rawResponse) = try await session.bytes(for: request)
        defer { bytes.task.cancel() }
        guard let response = rawResponse as? HTTPURLResponse else { throw ZooError.invalidResponse }
        if response.statusCode == 409 {
            try Task.checkCancellation()
            try await receive(Data(), response)
            return .complete
        }
        try Self.check(Data(), response)
        guard response.value(forHTTPHeaderField: "Content-Type")?.lowercased().contains("text/event-stream") == true else {
            // SSE非対応のプロキシではJSON応答を受け取り、呼び出し側がロングポーリングへ戻す。
            var data = Data()
            for try await byte in bytes {
                data.append(byte)
                guard data.count <= 64 * 1024 * 1024 else { throw ZooError.invalidResponse }
            }
            try Task.checkCancellation()
            try await receive(data, response)
            return .longPolling
        }
        try await withTaskCancellationHandler {
            var parser = ElectricSSEParser()
            var events: [ShapeEvent] = []
            var pendingBytes = 0
            var receivedBatch = false
            for try await byte in bytes {
                try Task.checkCancellation()
                guard let data = try parser.feed(byte) else { continue }
                guard events.count < 10_000 else { throw ZooError.invalidResponse }
                let event = try JSONDecoder().decode(ShapeEvent.self, from: data)
                events.append(event)
                pendingBytes += data.count
                guard pendingBytes <= 64 * 1024 * 1024 else { throw ZooError.invalidResponse }
                if event.headers.control == "must-refetch" {
                    try await receive(try JSONEncoder().encode(events), response)
                    return
                }
                guard event.headers.control == "up-to-date" else { continue }
                guard let lsn = event.headers.global_last_seen_lsn, UInt64(lsn) != nil,
                      let url = response.url else { throw ZooError.invalidResponse }
                var headers: [String: String] = [:]
                for name in ["electric-handle", "electric-cursor"] { headers[name] = response.value(forHTTPHeaderField: name) }
                headers["electric-offset"] = lsn + "_0"
                headers["electric-up-to-date"] = "true"
                guard let batchResponse = HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: headers) else {
                    throw ZooError.invalidResponse
                }
                try await receive(try JSONEncoder().encode(events), batchResponse)
                receivedBatch = true
                events.removeAll(keepingCapacity: true)
                pendingBytes = 0
            }
            // up-to-dateに到達していない末尾は保存せず、前の同期位置から再取得する。
            if !receivedBatch || !events.isEmpty || pendingBytes > 0 || parser.hasPendingEvent { throw ZooError.invalidResponse }
        } onCancel: { bytes.task.cancel() }
        return .complete
    }
}
