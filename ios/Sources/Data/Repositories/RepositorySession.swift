import Foundation

// A session invalidates every in-flight write before another account can become active.
@MainActor
final class RepositorySession {
    let api: APIClient
    private var valid = true
    init(api: APIClient) { self.api = api }
    func invalidate() { valid = false }
    func check() throws {
        guard valid, !Task.isCancelled else { throw CancellationError() }
    }
    func write<T: Decodable & Sendable>(_ path: String, method: String = "POST",
                                      query: [String: String] = [:], body: [String: JSONValue] = [:]) async throws -> T {
        try check()
        let result: T = try await api.write(path, method: method, query: query, body: body)
        try check()
        return result
    }
}
