import Foundation
import Synchronization
import Testing
import ZooCrypto
@testable import Zoo

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct BrowserAuthenticationTests {
    private final class Vault {
        var token: TokenSet?
        var saves = 0
    }

    private let stubID = UUID().uuidString
    private let settings = ConnectionSettings(server: "https://api.test.invalid", issuer: "https://id.test.invalid",
                                               clientID: "ios-test", resource: "https://api.test.invalid")

    private func withAuthentication(_ action: (Authentication, Vault) async throws -> Void) async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubProtocol.self]
        configuration.httpAdditionalHeaders = ["X-Zoo-Test-ID": stubID]
        let transport = HTTPTransport(session: URLSession(configuration: configuration))
        let vault = Vault()
        let auth = Authentication(settings: settings, transport: transport, loadToken: { _ in vault.token },
                                  saveToken: { token, _ in vault.token = token; vault.saves += 1 },
                                  deleteToken: { _ in vault.token = nil })
        defer { transport.session.invalidateAndCancel(); StubProtocol.install(nil, id: stubID) }
        try await action(auth, vault)
    }

    nonisolated private static var discovery: Data {
        Data(#"{"issuer":"https://id.test.invalid","authorization_endpoint":"https://id.test.invalid/authorize","token_endpoint":"https://id.test.invalid/token"}"#.utf8)
    }

    nonisolated private static func token(nonce: String, changes: [String: JSONValue] = [:]) throws -> Data {
        var claims: [String: JSONValue] = ["iss": .string("https://id.test.invalid"), "aud": .string("ios-test"),
            "exp": .number(Date().addingTimeInterval(3600).timeIntervalSince1970), "nonce": .string(nonce)]
        claims.merge(changes) { _, new in new }
        let idToken = "e30." + PasskeyCoding.base64URL(try JSONEncoder().encode(claims)) + ".sig"
        let response: [String: JSONValue] = ["id_token": .string(idToken), "access_token": .string("test-access"),
            "refresh_token": .string("test-refresh"), "token_type": .string("Bearer"), "expires_in": .number(3600)]
        return try JSONEncoder().encode(response)
    }

    @Test("パスキーの認証画面の後はコードをPKCEで一度だけ交換し、更新トークンを保存する")
    func browserLogin() async throws {
        let nonce = Mutex(""), verifier = Mutex(""), paths = Mutex<[String]>([])
        StubProtocol.install({ request in
            paths.withLock { $0.append(request.url!.path) }
            if request.url?.path == "/.well-known/openid-configuration" { return (200, [:], Self.discovery) }
            #expect(request.url?.path == "/token")
            #expect(request.httpMethod == "POST")
            #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
            let body = try StubProtocol.body(request)
            let expected = OAuthForm.form(["grant_type": "authorization_code", "code": "test+code&value",
                "code_verifier": verifier.withLock { $0 }, "client_id": "ios-test",
                "resource": "https://api.test.invalid", "redirect_uri": ReauthRequest.redirectURI])
            #expect(body == expected)
            return (200, [:], try Self.token(nonce: nonce.withLock { $0 }))
        }, id: stubID)
        try await withAuthentication { auth, vault in
            try await auth.login { request in
                nonce.withLock { $0 = request.nonce }
                verifier.withLock { $0 = request.verifier }
                let items = try #require(URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.queryItems)
                #expect(items.contains(URLQueryItem(name: "scope", value: "openid profile offline_access")))
                #expect(items.contains(URLQueryItem(name: "prompt", value: "consent")))
                #expect(!items.contains { $0.name == "max_age" })
                #expect(request.state.count == 43 && request.nonce.count == 43 && request.verifier.count == 43)
                return .code("test+code&value")
            }
            #expect(vault.saves == 1)
            #expect(vault.token?.refresh == "test-refresh")
            #expect(try await auth.accessToken() == "test-access")
            #expect(paths.withLock { $0 } == ["/.well-known/openid-configuration", "/token"])
        }
    }

    @Test("本人確認の応答が別の要求・利用者向けなら認証情報を保存しない", arguments: ["nonce", "iss", "aud", "exp"])
    func rejectsInvalidIDToken(claim: String) async throws {
        let nonce = Mutex("")
        StubProtocol.install({ request in
            if request.url?.path == "/.well-known/openid-configuration" { return (200, [:], Self.discovery) }
            let value: JSONValue = claim == "exp" ? .number(0) : .string("other")
            return (200, [:], try Self.token(nonce: nonce.withLock { $0 }, changes: [claim: value]))
        }, id: stubID)
        try await withAuthentication { auth, vault in
            await #expect(throws: ReauthError.self) {
                try await auth.login { request in nonce.withLock { $0 = request.nonce }; return .code("test-code") }
            }
            #expect(vault.saves == 0 && vault.token == nil)
            let hasSession = try auth.hasSession
            #expect(!hasSession)
        }
    }

    @Test("認証画面を閉じた場合と拒否された場合はコード交換をしない", arguments: [false, true])
    func cancelledOrDenied(cancelled: Bool) async throws {
        let requests = Mutex(0)
        StubProtocol.install({ request in
            requests.withLock { $0 += 1 }
            #expect(request.url?.path == "/.well-known/openid-configuration")
            return (200, [:], Self.discovery)
        }, id: stubID)
        try await withAuthentication { auth, vault in
            do {
                try await auth.login { _ in
                    if cancelled { throw CancellationError() }
                    return .error("access_denied")
                }
                Issue.record("取り消した認証が成功しました")
            } catch {
                #expect(cancelled ? error is CancellationError : error is ZooError)
            }
            #expect(vault.saves == 0 && vault.token == nil)
            #expect(requests.withLock { $0 } == 1)
        }
    }

    @Test("認証画面の応答前にログアウトした場合は古いコードを交換しない")
    func logoutDuringBrowser() async throws {
        StubProtocol.install({ request in
            #expect(request.url?.path == "/.well-known/openid-configuration")
            return (200, [:], Self.discovery)
        }, id: stubID)
        try await withAuthentication { auth, vault in
            await #expect(throws: CancellationError.self) {
                try await auth.login { _ in try auth.logout(); return .code("late-code") }
            }
            #expect(vault.saves == 0 && vault.token == nil)
        }
    }

    @Test("認可コード交換中にログアウトしても遅れたトークンを保存しない")
    func logoutDuringExchange() async throws {
        let requested = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        let nonce = Mutex("")
        defer { requested.continuation.finish(); release.continuation.finish() }
        StubProtocol.install({ request in
            if request.url?.path == "/.well-known/openid-configuration" { return (200, [:], Self.discovery) }
            requested.continuation.yield(())
            for await _ in release.stream { break }
            return (200, [:], try Self.token(nonce: nonce.withLock { $0 }))
        }, id: stubID)
        try await withAuthentication { auth, vault in
            let login = Task { try await auth.login { request in nonce.withLock { $0 = request.nonce }; return .code("test-code") } }
            defer { login.cancel() }
            var iterator = requested.stream.makeAsyncIterator()
            try #require(await iterator.next() != nil)
            try auth.logout()
            release.continuation.finish()
            await #expect(throws: CancellationError.self) { try await login.value }
            #expect(vault.saves == 0 && vault.token == nil)
        }
    }

    @Test("暗号鍵を解錠するパスキーのスロットがなければ選択画面を開かない")
    func noPasskeySlots() async throws {
        await #expect(throws: PasskeyError.noSlot) {
            try await SystemPasskeyClient().prfForUnlock(rpID: "id.test.invalid", slots: [])
        }
    }

    @Test("暗号化のPRF出力は32バイトだけを受け付ける", arguments: [0, 16, 31, 32, 33])
    func prfLength(count: Int) throws {
        let output = Data(repeating: 1, count: count)
        if count == 32 { #expect(try PasskeyCoding.checkedOutput(output) == output) }
        else { #expect(throws: PasskeyError.prfUnsupported) { try PasskeyCoding.checkedOutput(output) } }
    }
}
