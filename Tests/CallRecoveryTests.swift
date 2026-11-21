import Foundation
import Security
import Testing
@testable import Zoo

@MainActor
private final class MemoryCallSessionJournal: CallSessionJournal {
    var records: [CallSessionRecord] = []
    var saveError: Error?
    func load() throws -> [CallSessionRecord] { records }
    func save(_ records: [CallSessionRecord]) throws {
        if let saveError { throw saveError }
        self.records = records
    }
}

@MainActor @Suite("強制終了後の通話の回復", .timeLimit(.minutes(1)))
struct CallRecoveryTests {
    private let channel = Channel(id: "11111111-1111-4111-8111-111111111111", channel_name: "一般", is_private: false, is_dm: false)
    private var previous: CallSessionRecord {
        CallSessionRecord(channelID: channel.id, sessionID: "22222222-2222-4222-8222-222222222222")
    }

    private func store(_ client: FakeCallClient, journal: any CallSessionJournal) -> CallStore {
        CallStore(client: client, userID: "me", makeMedia: { FakeCallMedia() }, permission: { true }, sessionJournal: journal)
    }

    @Test("再起動後は前回の端末セッションだけを退出し、他の参加者がいる通話は表示する")
    func recoverAfterRelaunch() async throws {
        let journal = MemoryCallSessionJournal()
        try journal.record(previous)
        let client = FakeCallClient()
        client.beforeActiveCallsResult = { #expect(journal.records.isEmpty) }
        client.activeCallRows = [ActiveCallSummary(channel_id: "other-channel", participant_count: 1)]
        let reopened = store(client, journal: journal)
        defer { reopened.shutdown() }
        await reopened.refreshActiveCalls()
        #expect(client.commands == ["leave"])
        #expect(client.commandChannels == [channel.id])
        #expect(client.commandBodies.first?.1["session_id"] == .string(previous.sessionID))
        #expect(!reopened.hasActiveCall(in: channel.id))
        #expect(reopened.hasActiveCall(in: "other-channel"))
        #expect(reopened.channel == nil && !reopened.ready)
    }

    @Test("記録を永続化してから入室し、一覧更新では現在の通話を退出しない")
    func recordsBeforeJoining() async throws {
        let journal = MemoryCallSessionJournal()
        let client = FakeCallClient()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let calls = store(client, journal: journal)
        defer { calls.shutdown() }
        await withCheckedContinuation { done in
            client.delivered = { done.resume() }
            calls.start(channel: channel)
        }
        #expect(journal.records == [CallSessionRecord(channelID: channel.id, sessionID: try #require(client.sessions.first))])
        await calls.refreshActiveCalls()
        #expect(!client.commands.contains("leave"))
        #expect(calls.ready)
        let left = AsyncStream<Void>.makeStream()
        client.beforeActiveCallsResult = { left.continuation.yield(()) }
        calls.finish()
        for await _ in left.stream { break }
        left.continuation.finish()
        #expect(journal.records.isEmpty)
    }

    @Test("保存に失敗したら入室しない")
    func cannotPersistSession() async {
        let journal = MemoryCallSessionJournal()
        journal.saveError = ZooError.auth("保存失敗")
        let client = FakeCallClient()
        let calls = store(client, journal: journal)
        defer { calls.shutdown() }
        await calls.start(channel: channel)?.value
        #expect(!client.observed && calls.channel == nil && calls.error != nil)
    }

    @Test("退出通信が失敗しても記録を残し、次の更新で再試行する")
    func retryRecovery() async throws {
        let journal = MemoryCallSessionJournal()
        try journal.record(previous)
        let client = FakeCallClient()
        client.commandFailure = { _ in URLError(.notConnectedToInternet) }
        let calls = store(client, journal: journal)
        defer { calls.shutdown() }
        await calls.refreshActiveCalls()
        #expect(journal.records == [previous])
        client.commandFailure = nil
        await calls.refreshActiveCalls()
        #expect(journal.records.isEmpty && client.commands == ["leave", "leave"])
    }

    @Test("古い通話が残っている間は新しく入室せず、回復後に発信する")
    func recoveryBeforeNewCall() async throws {
        let journal = MemoryCallSessionJournal()
        try journal.record(previous)
        let client = FakeCallClient()
        client.commandFailure = { _ in URLError(.notConnectedToInternet) }
        let calls = store(client, journal: journal)
        defer { calls.shutdown() }
        await calls.start(channel: channel)?.value
        #expect(!client.observed && calls.channel == nil)
        #expect(journal.records.contains(previous))
        client.commandFailure = nil
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        await withCheckedContinuation { done in
            client.delivered = { done.resume() }
            calls.start(channel: channel)
        }
        #expect(calls.ready && !journal.records.contains(previous))
        #expect(journal.records.count == 1)
    }

    @Test("退出だけが失敗しても別チャンネルの通話一覧を更新する", arguments: [false, true])
    func listsCallsWhenRecoveryFails(storageFailure: Bool) async throws {
        let journal = MemoryCallSessionJournal()
        try journal.record(previous)
        let client = FakeCallClient()
        let calls = store(client, journal: journal)
        defer { calls.shutdown() }
        if storageFailure { journal.saveError = ZooError.auth("記録の削除失敗") }
        else { client.commandFailure = { _ in ZooError.rejected(500, "退出失敗") } }
        client.activeCallRows = [ActiveCallSummary(channel_id: "other-channel", participant_count: 1)]
        await calls.refreshActiveCalls()
        #expect(calls.hasActiveCall(in: "other-channel"))
        #expect(journal.records == [previous])
        client.activeCallRows = [ActiveCallSummary(channel_id: "new-channel", participant_count: 2)]
        await calls.refreshActiveCalls()
        #expect(!calls.hasActiveCall(in: "other-channel"))
        #expect(calls.hasActiveCall(in: "new-channel"))
    }

    @Test("退出済み・権限喪失は完了扱いにし、認証切れやサーバー障害は記録を残す", arguments: [403, 404, 401, 500])
    func recoveryErrors(status: Int) async throws {
        let journal = MemoryCallSessionJournal()
        try journal.record(previous)
        let client = FakeCallClient()
        client.commandFailure = { _ in status == 401 ? ZooError.expiredSession : ZooError.rejected(status, "テスト") }
        let calls = store(client, journal: journal)
        defer { calls.shutdown() }
        var expired = false
        calls.onExpiredSession = { expired = true }
        await calls.refreshActiveCalls()
        #expect(journal.records.isEmpty == (status == 403 || status == 404))
        #expect(expired == (status == 401))
    }

    @Test("通常終了の退出失敗も再起動後に回復する")
    func failedFinishSurvivesRelaunch() async throws {
        let journal = MemoryCallSessionJournal()
        let client = FakeCallClient()
        client.events = [.room(CallRoom(participants: [], max_participants: 6, ice_servers: []))]
        let calls = store(client, journal: journal)
        await withCheckedContinuation { done in
            client.delivered = { done.resume() }
            calls.start(channel: channel)
        }
        let recorded = try #require(journal.records.first)
        let attempted = AsyncStream<Void>.makeStream()
        client.commandFailure = { $0 == "leave" ? URLError(.networkConnectionLost) : nil }
        client.commandObserved = { if $0 == "leave" { attempted.continuation.yield(()) } }
        calls.finish()
        for await _ in attempted.stream { break }
        attempted.continuation.finish()
        calls.shutdown()
        #expect(journal.records == [recorded])
        let restartedClient = FakeCallClient()
        let reopened = store(restartedClient, journal: journal)
        defer { reopened.shutdown() }
        await reopened.refreshActiveCalls()
        #expect(journal.records.isEmpty)
        #expect(restartedClient.commandBodies.first?.1["session_id"] == .string(recorded.sessionID))
    }

    @Test("ログアウトで回復を中断したら記録を残し、古い一覧を反映しない")
    func logoutDuringRecovery() async throws {
        let journal = MemoryCallSessionJournal()
        try journal.record(previous)
        let client = FakeCallClient()
        let started = AsyncStream<Void>.makeStream()
        client.beforeCommand = { _ in
            started.continuation.yield(())
            try await Task.sleep(for: .seconds(86_400))
        }
        client.beforeActiveCallsResult = { Issue.record("ログアウト後に一覧を取得してはいけません") }
        let calls = store(client, journal: journal)
        let pending = Task { await calls.refreshActiveCalls() }
        for await _ in started.stream { break }
        started.continuation.finish()
        calls.shutdown()
        await pending.value
        #expect(journal.records == [previous] && calls.activeChannelIDs.isEmpty)
    }

    @Test("古い退出応答は後から記録された別セッションを消さない")
    func lateLeaveResult() async throws {
        let journal = MemoryCallSessionJournal()
        try journal.record(previous)
        let replacement = CallSessionRecord(channelID: channel.id, sessionID: UUID().uuidString)
        let client = FakeCallClient()
        client.beforeCommand = { _ in try journal.record(replacement) }
        let calls = store(client, journal: journal)
        defer { calls.shutdown() }
        await calls.refreshActiveCalls()
        #expect(journal.records == [replacement])
    }

    @Test("同時に一覧更新しても古いセッションの退出を重ねない")
    func coalescesRecovery() async throws {
        let journal = MemoryCallSessionJournal()
        try journal.record(previous)
        let client = FakeCallClient()
        let started = AsyncStream<Void>.makeStream()
        let release = AsyncStream<Void>.makeStream()
        client.beforeCommand = { _ in
            started.continuation.yield(())
            for await _ in release.stream { break }
        }
        let calls = store(client, journal: journal)
        defer { calls.shutdown(); started.continuation.finish(); release.continuation.finish() }
        let first = Task { await calls.refreshActiveCalls() }
        for await _ in started.stream { break }
        let second = Task { await calls.refreshActiveCalls() }
        await Task.yield()
        release.continuation.yield(())
        await first.value
        await second.value
        #expect(client.commands == ["leave"] && journal.records.isEmpty)
    }

    @Test("端末の通話記録は再生成しても読め、アカウントと接続先を混同しない")
    func persistentScope() throws {
        let service = "works.zli.zoo.tests.call-sessions." + UUID().uuidString
        let original = KeychainCallSessionJournal(account: "server-A:user-A", service: service)
        defer { try? original.save([]) }
        try original.record(previous)
        let reopened = KeychainCallSessionJournal(account: "server-A:user-A", service: service)
        #expect(try reopened.load() == [previous])
        #expect(try KeychainCallSessionJournal(account: "server-A:user-B", service: service).load().isEmpty)
        #expect(try KeychainCallSessionJournal(account: "server-B:user-A", service: service).load().isEmpty)
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service, kSecAttrAccount as String: "server-A:user-A",
            kSecReturnAttributes as String: true]
        var result: CFTypeRef?
        #expect(SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess)
        #expect((result as? [String: Any])?[kSecAttrAccessible as String] as? String == kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
    }
}
