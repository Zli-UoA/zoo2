import Foundation
import Testing
@testable import Zoo

@MainActor
@Suite("入力中通知の通信", .timeLimit(.minutes(1)))
struct TypingAPITests {
    private let channelID = "11111111-1111-4111-8111-111111111111"

    @Test("会話とスレッドの入力者を取得する", arguments: [nil, "22222222-2222-4222-8222-222222222222"] as [String?])
    func readsScope(parentID: String?) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let channelID = channelID
        harness.stub { request in
            let url = try #require(request.url)
            #expect(url.path == "/api/channels/\(channelID)/typing")
            #expect(request.httpMethod == "GET")
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-access")
            #expect(request.timeoutInterval == 5)
            let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
            #expect(query == parentID.map { [URLQueryItem(name: "parent_id", value: $0)] } ?? [])
            return (200, [:], Data(#"[{"id":"相手","display_name":"さくら"}]"#.utf8))
        }
        let users = try await harness.client.typingUsers(channelID: channelID, parentID: parentID)
        #expect(users == [TypingUser(id: "相手", display_name: "さくら")])
    }

    @Test("開始と停止は本文を含めず同じセッションで送る", arguments: [false, true])
    func writesPresence(active: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let channelID = channelID
        let parentID = UUID().uuidString.lowercased()
        let sessionID = UUID()
        harness.stub { request in
            #expect(request.url?.path == "/api/channels/\(channelID)/typing")
            #expect(request.httpMethod == "POST")
            #expect(request.timeoutInterval == 5)
            let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
            #expect(body == ["parent_id": .string(parentID), "session_id": .string(sessionID.uuidString.lowercased()), "active": .bool(active)])
            return (200, [:], Data(#"{"ok":true}"#.utf8))
        }
        try await harness.client.setTyping(channelID: channelID, parentID: parentID, sessionID: sessionID, active: active)
    }

    @Test("通常の会話では親IDをnullとして送る")
    func writesRootScope() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            let body = try JSONDecoder().decode([String: JSONValue].self, from: StubProtocol.body(request))
            #expect(body["parent_id"] == .null)
            #expect(body.count == 3)
            return (200, [:], Data(#"{"ok":true}"#.utf8))
        }
        try await harness.client.setTyping(channelID: channelID, parentID: nil, sessionID: UUID(), active: true)
    }

    @Test("不正な送り先は通信前に拒否する")
    func rejectsInvalidScope() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in
            Issue.record("不正な送り先で通信してはいけない")
            return (200, [:], Data("[]".utf8))
        }
        await #expect(throws: ZooError.self) { try await harness.client.typingUsers(channelID: "../other", parentID: nil) }
        await #expect(throws: ZooError.self) {
            try await harness.client.setTyping(channelID: channelID, parentID: "invalid", sessionID: UUID(), active: true)
        }
    }
}

@MainActor
private final class TypingStub: TypingClient {
    struct Write {
        let parentID: String?
        let sessionID: UUID
        let active: Bool
    }
    var users: [TypingUser] = []
    var error: Error?
    var writes: [Write] = []
    var reads: [String?] = []
    var read: ((String?) async throws -> [TypingUser])?
    var write: ((Bool) async throws -> Void)?

    func typingUsers(channelID: String, parentID: String?) async throws -> [TypingUser] {
        reads.append(parentID)
        if let read { return try await read(parentID) }
        if let error { throw error }
        return users
    }

    func setTyping(channelID: String, parentID: String?, sessionID: UUID, active: Bool) async throws {
        writes.append(Write(parentID: parentID, sessionID: sessionID, active: active))
        try await write?(active)
    }
}

@MainActor
@Suite("入力中の状態管理", .timeLimit(.minutes(1)))
struct TypingStoreTests {
    private func store(_ client: TypingStub, idle: Duration = .seconds(3), interval: Duration = .seconds(2)) -> TypingStore {
        TypingStore(client: client, channelID: "会話", userID: "自分", pollInterval: .milliseconds(10),
                    idleTimeout: idle, sendInterval: interval)
    }

    private func until(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now.advanced(by: .seconds(3))
        while !condition(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        try #require(condition(), "期待する状態に遷移する")
    }

    @Test("自分と重複を除外し、複数人を表示する")
    func displaysOtherPeople() async throws {
        let client = TypingStub()
        client.users = [TypingUser(id: "自分", display_name: "あなた"), TypingUser(id: "1", display_name: "さくら"),
                        TypingUser(id: "2", display_name: "あおい"), TypingUser(id: "1", display_name: "さくら")]
        let store = store(client)
        let task = Task { await store.run(parentID: nil) }
        defer { task.cancel(); store.stop() }
        try await until { store.users.count == 2 }
        #expect(store.indicator(parentID: nil) == "さくら、あおいが入力中…")
        #expect(store.indicator(parentID: "別スレッド") == nil)
        client.users.append(TypingUser(id: "3", display_name: "みどり"))
        try await until { store.users.count == 3 }
        #expect(store.indicator(parentID: nil) == "さくら、あおい、ほか1人が入力中…")
        client.users = []
        try await until { store.users.isEmpty }
        #expect(store.indicator(parentID: nil) == nil)
    }

    @Test("通信失敗とキャンセルで古い表示を消す")
    func clearsOnFailureAndCancellation() async throws {
        let client = TypingStub()
        client.users = [TypingUser(id: "1", display_name: "さくら")]
        let store = store(client)
        let task = Task { await store.run(parentID: nil) }
        defer { task.cancel(); store.stop() }
        try await until { !store.users.isEmpty }
        client.error = ZooError.invalidResponse
        try await until { store.users.isEmpty }
        client.error = nil
        try await until { !store.users.isEmpty }
        task.cancel()
        await task.value
        #expect(store.users.isEmpty)
        store.activity(true, parentID: nil)
        #expect(client.writes.isEmpty)
    }

    @Test("切り替え前の遅い応答を反映しない")
    func ignoresOldResponse() async throws {
        let client = TypingStub()
        var reply: CheckedContinuation<[TypingUser], Never>?
        client.read = { parent in
            if parent == nil { return await withCheckedContinuation { reply = $0 } }
            return [TypingUser(id: "2", display_name: "あおい")]
        }
        let store = store(client)
        let root = Task { await store.run(parentID: nil) }
        defer {
            root.cancel(); store.stop()
            let pending = reply; reply = nil
            pending?.resume(returning: [])
        }
        try await until { reply != nil }
        let thread = Task { await store.run(parentID: "スレッド") }
        defer { root.cancel(); thread.cancel(); store.stop() }
        try await until { store.indicator(parentID: "スレッド") != nil }
        let pending = reply; reply = nil
        pending?.resume(returning: [TypingUser(id: "1", display_name: "古い応答")])
        await root.value
        #expect(store.indicator(parentID: "スレッド") == "あおいが入力中…")
        #expect(store.indicator(parentID: nil) == nil)
    }

    @Test("連打を間引き、入力停止時に通知を解除する")
    func throttlesAndStopsWhenIdle() async throws {
        let client = TypingStub()
        let store = store(client, idle: .milliseconds(80))
        let task = Task { await store.run(parentID: nil) }
        defer { task.cancel(); store.stop() }
        try await until { !client.reads.isEmpty }
        store.activity(true, parentID: nil)
        try await until { client.writes.count == 1 }
        for _ in 0..<20 { store.activity(true, parentID: nil) }
        try await until { client.writes.count == 2 }
        #expect(client.writes.map(\.active) == [true, false])
        #expect(client.writes[0].sessionID == client.writes[1].sessionID)
    }

    @Test("開始が遅れても停止を追い越さず、最新状態だけを送る")
    func serializesDelayedWrites() async throws {
        let client = TypingStub()
        var complete: CheckedContinuation<Void, Never>?
        client.write = { active in
            if active { await withCheckedContinuation { complete = $0 } }
        }
        let store = store(client, interval: .zero)
        let task = Task { await store.run(parentID: "スレッド") }
        defer {
            task.cancel(); store.stop()
            let pending = complete; complete = nil
            pending?.resume()
        }
        try await until { !client.reads.isEmpty }
        store.activity(true, parentID: "スレッド")
        try await until { complete != nil }
        for _ in 0..<20 { store.activity(true, parentID: "スレッド") }
        store.activity(false, parentID: "スレッド")
        #expect(client.writes.map(\.active) == [true])
        let pending = complete; complete = nil
        pending?.resume()
        try await until { client.writes.count == 2 }
        #expect(client.writes.map(\.active) == [true, false])
        #expect(client.writes.allSatisfy { $0.parentID == "スレッド" })
    }

    @Test("画面を離れると停止し、再訪は別セッションを使う")
    func stopsOnLeavingAndIsolatesVisits() async throws {
        let client = TypingStub()
        let store = store(client)
        let first = Task { await store.run(parentID: nil) }
        defer { first.cancel(); store.stop() }
        try await until { !client.reads.isEmpty }
        store.activity(true, parentID: "別スレッド")
        #expect(client.writes.isEmpty)
        store.activity(true, parentID: nil)
        try await until { client.writes.count == 1 }
        first.cancel()
        await first.value
        try await until { client.writes.count == 2 }
        #expect(client.writes.map(\.active) == [true, false])
        let readsBeforeReturn = client.reads.count
        let second = Task { await store.run(parentID: nil) }
        defer { second.cancel(); store.stop() }
        try await until { client.reads.count > readsBeforeReturn }
        store.activity(true, parentID: nil)
        try await until { client.writes.count == 3 }
        #expect(client.writes[2].sessionID != client.writes[0].sessionID)
    }
}
