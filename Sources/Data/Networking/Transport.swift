import Foundation

struct ConnectionSettings: Codable, Equatable, Sendable {
    var server = "https://zli.chat"
    var issuer = "https://account.zli.works/api/auth"
    var clientID = "UPPfdYapcgiBgnfZIoGvFuJMAEIhXQOV"
    var resource = "https://zli.chat/api"
    var scope = "openid profile offline_access"
    static let callback = "https://zli.chat/ios/oauth/callback"

    static func secureURL(_ raw: String) throws -> URL {
        guard let url = URL(string: raw), url.scheme == "https", url.host != nil,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil else {
            throw ZooError.invalidConfiguration
        }
        return url
    }

    func validate() throws {
        _ = try Self.secureURL(server)
        _ = try Self.secureURL(issuer)
        guard !clientID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw ZooError.auth("管理者が登録した iOS 用 OAuth クライアント ID を設定してください。")
        }
    }

    var storageKey: String { "\(server)|\(issuer)|\(clientID)|\(resource)|\(scope)" }
}

final class NoRedirects: NSObject, URLSessionTaskDelegate, Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest,
                    completionHandler: @escaping @Sendable (URLRequest?) -> Void) {
        // Tokens are never forwarded to redirect destinations.
        completionHandler(nil)
    }
}

final class HTTPTransport: Sendable {
    let session: URLSession

    init(session: URLSession? = nil) {
        let config = URLSessionConfiguration.ephemeral
        config.urlCache = nil
        config.httpMaximumConnectionsPerHost = 12
        config.timeoutIntervalForRequest = 75
        self.session = session ?? URLSession(configuration: config, delegate: NoRedirects(), delegateQueue: nil)
    }

    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let (data, response) = try await session.data(for: request)
        guard let response = response as? HTTPURLResponse else { throw ZooError.invalidResponse }
        return (data, response)
    }

    static func check(_ data: Data, _ response: HTTPURLResponse) throws {
        if response.statusCode == 401 { throw ZooError.expiredSession }
        guard (200..<300).contains(response.statusCode) else {
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            let message = json?["message"] as? String ?? json?["error"] as? String ?? ""
            throw ZooError.rejected(response.statusCode, String(message.prefix(240)))
        }
    }
}

@MainActor
final class APIClient {
    let settings: ConnectionSettings
    let auth: Authentication
    let transport: HTTPTransport

    init(settings: ConnectionSettings, auth: Authentication, transport: HTTPTransport = HTTPTransport()) {
        self.settings = settings
        self.auth = auth
        self.transport = transport
    }

    func request(_ path: String, method: String = "GET", query: [String: String] = [:],
                 body: [String: JSONValue]? = nil) async throws -> URLRequest {
        let base = try ConnectionSettings.secureURL(settings.server)
        guard var parts = URLComponents(url: base.appendingPathComponent(path), resolvingAgainstBaseURL: false) else {
            throw ZooError.invalidConfiguration
        }
        if !query.isEmpty { parts.queryItems = query.sorted { $0.key < $1.key }.map { URLQueryItem(name: $0.key, value: $0.value) } }
        guard let url = parts.url else { throw ZooError.invalidConfiguration }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("Bearer \(try await auth.accessToken())", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let body {
            request.httpBody = try JSONEncoder().encode(body)
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        return request
    }

    func data(_ path: String, method: String = "GET", query: [String: String] = [:],
              body: [String: JSONValue]? = nil) async throws -> Data {
        let request = try await request(path, method: method, query: query, body: body)
        let (data, response) = try await transport.send(request)
        try HTTPTransport.check(data, response)
        return data
    }

    func get<T: Decodable & Sendable>(_ path: String) async throws -> T {
        try JSONDecoder().decode(T.self, from: await data(path))
    }

    func write<T: Decodable & Sendable>(_ path: String, method: String = "POST",
                                       query: [String: String] = [:], body: [String: JSONValue] = [:]) async throws -> T {
        let bytes = try await data(path, method: method, query: query, body: body)
        return try JSONDecoder().decode(Envelope<T>.self, from: bytes).data
    }
}
