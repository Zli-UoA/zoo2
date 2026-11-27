import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor
@Suite("全文検索")
struct GlobalSearchTests {
    nonisolated private static let channelID = "11111111-1111-4111-8111-111111111111"
    nonisolated private static let parentID = "22222222-2222-4222-8222-222222222222"
    nonisolated private static let replyID = "33333333-3333-4333-8333-333333333333"
    nonisolated private static let userID = "44444444-4444-4444-8444-444444444444"

    nonisolated private static func messageJSON(_ id: String, parent: String? = nil, author: Bool = true) -> String {
        let parentValue = parent.map { "\"\($0)\"" } ?? "null"
        let authorValue = author ? #""author":{"id":"u","user_name":"bob","display_name":"Bob","avatar_url":null,"is_bot":false},"# : ""
        return #"{"id":"\#(id)","kind":"message","channel_id":"\#(channelID)","channel_name":"general","is_dm":false,"parent_id":\#(parentValue),"user_id":"u",\#(authorValue)"emojis":{"stamp":{"url":"/stamp.png"}},"snippet":"会議の資料","created_at":"2026-11-21T01:02:03.456"}"#
    }

    nonisolated private static let fileJSON = #"{"id":"att-1","kind":"file","message_id":"msg-9","channel_id":"\#(channelID)","channel_name":"general","is_dm":false,"user_id":"u","file_name":"資料.pdf","content_type":"application/pdf","file_size":2048,"width":null,"height":null,"is_sensitive":false,"created_at":"2026-11-21 01:02:03.456"}"#

    nonisolated private static func page(_ items: [String], next: String? = nil, until: String? = "2026-04") -> Data {
        let cursor = next.map { "\"\($0.replacingOccurrences(of: "\"", with: "\\\""))\"" } ?? "null"
        let month = until.map { "\"\($0)\"" } ?? "null"
        return Data(#"{"items":[\#(items.joined(separator: ","))],"next_cursor":\#(cursor),"searched_until":\#(month)}"#.utf8)
    }

    private func store(_ harness: ChatHarness) -> GlobalSearchStore {
        GlobalSearchStore(client: harness.client)
    }

    @Test("会話失効は取得済みの検索結果を消し、ログアウトは検索語も消す")
    func workspaceClearsSearchResults() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in (200, [:], Self.page([Self.messageJSON("m1")])) }
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: Self.userID, user_name: "test", display_name: "利用者"), recentEmojiDefaults: nil)
        let search = workspace.search
        search.text = "会議"
        search.submit()
        await search.inflight?.value
        try #require(search.items.count == 1)
        workspace.revokeChannel(Self.channelID)
        #expect(search.items.isEmpty)
        workspace.shutdown()
        #expect(search.text.isEmpty && search.items.isEmpty && search.phase == .idle)
        search.submit()
        #expect(search.inflight == nil)
    }

    @Test("検索の期限を超えたら遅い応答で結果を戻さない")
    func searchDeadlineIgnoresLateResponse() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let release = AsyncStream<Void>.makeStream()
        defer { release.continuation.finish() }
        harness.stub { _ in
            for await _ in release.stream { break }
            return (200, [:], Self.page([Self.messageJSON("late")]))
        }
        #expect(GlobalSearchStore.timeout == .seconds(20))
        let search = GlobalSearchStore(client: harness.client, timeout: .milliseconds(20))
        search.text = "会議"; search.submit()
        let request = search.inflight
        for _ in 0..<100 where search.isSearching { try await Task.sleep(for: .milliseconds(10)) }
        #expect(search.phase == .failed("検索できませんでした。"))
        release.continuation.finish()
        await request?.value
        #expect(search.items.isEmpty)
        #expect(search.phase == .failed("検索できませんでした。"))
    }

    @Test("検索中の認証切れをワークスペースへ伝えて結果を捨てる")
    func searchReportsExpiredSession() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in (401, [:], Data()) }
        let workspace = WorkspaceStore(client: harness.client,
            user: Person(id: Self.userID, user_name: "test", display_name: "利用者"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        var expired = 0
        workspace.onExpiredSession = { expired += 1 }
        workspace.search.text = "会議"; workspace.search.submit()
        await workspace.search.inflight?.value
        #expect(expired == 1)
        #expect(workspace.search.text.isEmpty && workspace.search.items.isEmpty)
    }

    @Test("メッセージとファイルを読み、authorがなくても読める")
    func decodesItems() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in (200, [:], Self.page([Self.messageJSON("m1", author: false), Self.messageJSON("m2", parent: Self.parentID), Self.fileJSON])) }
        let search = store(harness)
        search.text = "会議"
        search.submit()
        await search.inflight?.value
        #expect(search.phase == .loaded)
        #expect(search.items.map(\.rowID) == ["message-m1", "message-m2", "file-att-1"])
        #expect(search.items[0].author == nil)
        #expect(search.items[1].destination == SearchDestination(channelID: Self.channelID, messageID: "m2", parentID: Self.parentID))
        let file = search.items[2]
        #expect(file.isFile && file.file_size == 2048 && file.createdDate != nil)
        #expect(file.destination == SearchDestination(channelID: Self.channelID, messageID: "msg-9", parentID: nil))
        #expect(search.searchedUntil == "2026-04")
    }

    @Test("種類を明示して送り、カーソルは記号を含めても元の文字列で届く")
    func sendsTypeAndCursor() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let cursor = #"{"m":24323,"at":"a+b&c#d=e","id":"x"}"#
        let requests = Mutex<[[String: String]]>([])
        harness.stub { request in
            let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            requests.withLock { $0.append(Dictionary(items.map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { $1 })) }
            return (200, [:], Self.page([Self.messageJSON("m1")], next: requests.withLock { $0.count } == 1 ? cursor : nil))
        }
        let search = store(harness)
        search.text = "会議 資料"
        search.submit()
        await search.inflight?.value
        search.loadOlder()
        await search.inflight?.value
        let sent = requests.withLock { $0 }
        #expect(sent.count == 2)
        #expect(sent[0]["type"] == "all" && sent[0]["q"] == "会議 資料" && sent[0]["cursor"] == nil && sent[0]["limit"] == "20")
        #expect(sent[1]["cursor"] == cursor)
        search.select(.files)
        await search.inflight?.value
        #expect(requests.withLock { $0.last?["type"] } == "files")
    }

    @Test("続きを読んでいる最中の再実行は二重に取得しない")
    func olderDoesNotDoubleFetch() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let count = Mutex(0)
        let gate = AsyncStream<Void>.makeStream()
        harness.stub { _ in
            let n = count.withLock { $0 += 1; return $0 }
            if n == 2 { for await _ in gate.stream { break } }
            return (200, [:], Self.page([Self.messageJSON("m\(n)")], next: n < 3 ? "c\(n)" : nil))
        }
        let search = store(harness)
        search.text = "a"
        search.submit()
        await search.inflight?.value
        search.loadOlder()
        search.loadOlder()
        search.loadOlder()
        gate.continuation.yield(())
        await search.inflight?.value
        #expect(count.withLock { $0 } == 2)
        #expect(search.items.count == 2)
    }

    @Test("語や種類を変えると前の取得を取り消し、古い応答の結果を捨てる")
    func staleResponseIsDropped() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let gate = AsyncStream<Void>.makeStream()
        let started = AsyncStream<String>.makeStream()
        harness.stub { request in
            let q = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "q" }?.value
            started.continuation.yield(q ?? "")
            if q == "old" {
                for await _ in gate.stream { break }
                return (200, [:], Self.page([Self.messageJSON("stale")]))
            }
            return (200, [:], Self.page([Self.messageJSON("fresh")]))
        }
        var iterator = started.stream.makeAsyncIterator()
        let search = store(harness)
        search.text = "old"
        search.submit()
        let stale = search.inflight
        // 旧い要求が通信を始めてから、語を変える。
        #expect(await iterator.next() == "old")
        search.text = "new"
        search.submit()
        #expect(await iterator.next() == "new")
        await search.inflight?.value
        gate.continuation.yield(())
        await stale?.value
        #expect(search.items.map(\.id) == ["fresh"])
        #expect(search.phase == .loaded)
    }

    @Test("失敗後は再試行で読み直し、続きの失敗では読んだ結果を残す")
    func retryAfterFailure() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let calls = Mutex(0)
        harness.stub { _ in
            let n = calls.withLock { $0 += 1; return $0 }
            if n == 1 || n == 3 { return (500, [:], Data()) }
            return (200, [:], Self.page([Self.messageJSON("m\(n)")], next: n == 2 ? "c" : nil))
        }
        let search = store(harness)
        search.text = "a"
        search.submit()
        await search.inflight?.value
        guard case .failed = search.phase else { Issue.record("失敗になっていない"); return }
        search.retry()
        await search.inflight?.value
        #expect(search.phase == .loaded && search.items.map(\.id) == ["m2"])
        search.loadOlder()
        await search.inflight?.value
        guard case .failed = search.phase else { Issue.record("続きの失敗になっていない"); return }
        #expect(search.items.map(\.id) == ["m2"])
        search.retry()
        await search.inflight?.value
        #expect(search.phase == .loaded && search.items.map(\.id) == ["m2", "m4"])
    }

    @Test("語数と文字数をサーバーと同じ数え方で検証する")
    func validatesTerms() {
        #expect(GlobalSearchQuery.validate("  ") == nil)
        #expect(GlobalSearchQuery.validate("a A a b") == .success("a b"))
        #expect(GlobalSearchQuery.validate("a b c d e f") == .failure(.tooManyTerms))
        #expect(GlobalSearchQuery.validate("a a a a a a b") == .success("a b"))
        #expect(GlobalSearchQuery.validate(String(repeating: "あ", count: 100)) != nil)
        #expect(GlobalSearchQuery.validate(String(repeating: "あ", count: 101)) == .failure(.termTooLong))
        #expect(GlobalSearchQuery.validate("🇯🇵" + String(repeating: "x", count: 98)) == .success("🇯🇵" + String(repeating: "x", count: 98)))
    }

    @Test("検証に失敗した語は送信しない")
    func invalidQueryIsNotSent() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let calls = Mutex(0)
        harness.stub { _ in calls.withLock { $0 += 1 }; return (200, [:], Self.page([])) }
        let search = store(harness)
        search.text = "a b c d e f"
        search.submit()
        #expect(search.inflight == nil && calls.withLock { $0 } == 0)
        #expect(search.validationError != nil && search.phase == .idle)
    }

    @Test("親も返信も未取得のまま、親のスレッドを開いて返信へ移動する")
    func opensReplyWithUnloadedParent() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let channel = Channel(id: Self.channelID, channel_name: "一般", is_private: false, is_dm: false)
        let bytes = try JSONEncoder().encode(channel)
        harness.stub { _ in (200, [:], bytes) }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: Self.userID, user_name: "you", display_name: "あなた"))
        let destination = SearchDestination(channelID: Self.channelID, messageID: Self.replyID, parentID: Self.parentID)
        #expect(await workspace.openSearchResult(destination))
        #expect(workspace.selectedTab == .channels)
        #expect(workspace.channelPath == [.channel(Self.channelID),
                                          .thread(channelID: Self.channelID, parentID: Self.parentID, focusComposer: false)])
        #expect(workspace.conversation(for: channel).focusedMessageID == Self.replyID)
    }

    private static func tokyo() -> Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "Asia/Tokyo")!
        return calendar
    }

    @Test("期間は端末の暦の日の0時を、オフセット付きで送る")
    func resolvesPeriods() throws {
        let calendar = Self.tokyo()
        let now = try #require(calendar.date(from: DateComponents(year: 2026, month: 11, day: 24, hour: 15)))
        var filters = SearchFilters()
        #expect(filters.resolved(now: now, calendar: calendar).after == nil)
        filters.period = .last7
        #expect(filters.resolved(now: now, calendar: calendar).after == "2026-11-18T00:00:00+09:00")
        filters.period = .last30
        #expect(filters.resolved(now: now, calendar: calendar).after == "2026-10-26T00:00:00+09:00")
        filters.period = .custom
        filters.customAfter = try #require(calendar.date(from: DateComponents(year: 2026, month: 11, day: 1, hour: 20)))
        filters.customEnd = try #require(calendar.date(from: DateComponents(year: 2026, month: 11, day: 30, hour: 3)))
        let custom = filters.resolved(now: now, calendar: calendar)
        #expect(custom.after == "2026-11-01T00:00:00+09:00" && custom.before == "2026-12-01T00:00:00+09:00")
        filters.customAfter = try #require(calendar.date(from: DateComponents(year: 2026, month: 12, day: 2)))
        #expect(filters.hasReversedDays(calendar: calendar))
        #expect(filters.resolved(now: now, calendar: calendar).after == nil)
    }

    @Test("絞り込みを送り、変えると取得し直す")
    func sendsFiltersAndRefetches() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let requests = Mutex<[[String: String]]>([])
        harness.stub { request in
            let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            requests.withLock { $0.append(Dictionary(items.map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { $1 })) }
            return (200, [:], Self.page([Self.messageJSON("m1")]))
        }
        let search = store(harness)
        search.text = "会議"
        search.submit()
        await search.inflight?.value
        var filters = SearchFilters()
        filters.senders = ["u1", "u2"]
        filters.channels = ["c1"]
        filters.involvingMe = true
        filters.excludeBots = true
        search.setFilters(filters)
        await search.inflight?.value
        let sent = try #require(requests.withLock { $0.last })
        #expect(sent["user_id"] == "u1,u2" && sent["channel_id"] == "c1")
        #expect(sent["involving_me"] == "true" && sent["exclude_bots"] == "true")
        #expect(requests.withLock { $0.count } == 2 && requests.withLock { $0[0]["user_id"] } == nil)
        search.setFilters(SearchFilters())
        await search.inflight?.value
        #expect(requests.withLock { $0.count } == 3 && requests.withLock { $0[2]["involving_me"] } == nil)
    }

    @Test("開始日が終了日より後なら取得し直さない")
    func reversedDaysDoNotSearch() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let calls = Mutex(0)
        harness.stub { _ in calls.withLock { $0 += 1 }; return (200, [:], Self.page([])) }
        let search = store(harness)
        search.text = "a"
        search.submit()
        await search.inflight?.value
        var filters = SearchFilters()
        filters.period = .custom
        filters.customAfter = Date()
        filters.customEnd = Date(timeIntervalSinceNow: -3 * 86400)
        search.setFilters(filters)
        #expect(search.inflight == nil && calls.withLock { $0 } == 1)
        #expect(search.phase == .idle && search.items.isEmpty)
        // 同じ語の送信や種類の変更からも、期間なしの検索を送らない。
        search.submit()
        search.select(.files)
        #expect(search.inflight == nil && calls.withLock { $0 } == 1)
    }

    @Test("閲覧権限を失う前に送った取得の応答でも、そのチャンネルの結果を戻さない")
    func revokedChannelIsNotRestored() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let gate = AsyncStream<Void>.makeStream()
        let started = AsyncStream<Void>.makeStream()
        harness.stub { _ in
            started.continuation.yield(())
            for await _ in gate.stream { break }
            return (200, [:], Self.page([Self.messageJSON("m1")]))
        }
        let search = store(harness)
        search.text = "a"
        search.submit()
        var iterator = started.stream.makeAsyncIterator()
        // 通信が始まってから、権限を失う。
        try #require(await iterator.next() != nil)
        search.revokeChannel(Self.channelID)
        gate.continuation.yield(())
        await search.inflight?.value
        #expect(search.phase == .loaded && search.items.isEmpty)
    }

    @Test("取得済みの同じ語を明示的に送信すると取り直す")
    func resubmitRefetches() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let calls = Mutex(0)
        harness.stub { _ in calls.withLock { $0 += 1 }; return (200, [:], Self.page([Self.messageJSON("m1")])) }
        let search = store(harness)
        search.text = "a"
        search.submit()
        await search.inflight?.value
        search.submit()
        await search.inflight?.value
        #expect(calls.withLock { $0 } == 2 && search.items.count == 1)
    }

    @Test("検索タブを離れたら、遅れて届いた応答でタブを奪わない")
    func leavingSearchTabCancelsOpening() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let channel = Channel(id: Self.channelID, channel_name: "一般", is_private: false, is_dm: false)
        let bytes = try JSONEncoder().encode(channel)
        let gate = AsyncStream<Void>.makeStream()
        let started = AsyncStream<Void>.makeStream()
        harness.stub { _ in
            started.continuation.yield(())
            for await _ in gate.stream { break }
            return (200, [:], bytes)
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: Self.userID, user_name: "you", display_name: "あなた"))
        workspace.selectedTab = .search
        let open = Task { await workspace.openSearchResultInTab(SearchDestination(channelID: Self.channelID, messageID: Self.replyID, parentID: nil)) }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        workspace.selectedTab = .profile
        gate.continuation.yield(())
        #expect(await open.value == false)
        #expect(workspace.selectedTab == .profile && workspace.channelPath.isEmpty)
    }
}
