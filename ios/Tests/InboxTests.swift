import Foundation
import Synchronization
import Testing
@testable import Zoo

@MainActor
@Suite("アプリ内通知", .timeLimit(.minutes(1)))
struct InboxTests {
    @Test("更新中に予約した次ページは更新元のキャンセル後も取得する")
    func pendingPageSurvivesRefreshCancellation() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let reads = Mutex((head: 0, older: 0))
        harness.stub { request in
            if request.url?.path == "/api/shapes/notifications" {
                return (200, ["electric-handle": "notices", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            }
            #expect(request.url?.path == "/api/activitypub/notifications")
            let older = request.url?.query?.contains("before=") == true
            let head = reads.withLock { value in
                if older { value.older += 1 } else { value.head += 1 }
                return value.head
            }
            if !older, head == 2 {
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                throw CancellationError()
            }
            let row: [String: Any] = ["id": older ? "old" : "new", "actor_id": "https://remote.test/user/member",
                "display_name": "メンバー", "kind": "reply", "activity_id": older ? "old" : "new",
                "created_at": older ? "2026-11-18T09:00:00Z" : "2026-11-18T10:00:00Z", "is_read": false, "pending": false]
            return (200, [:], try JSONSerialization.data(withJSONObject: ["items": [row], "unread_count": 2,
                "next_cursor": older ? NSNull() as Any : "new" as Any]))
        }
        let inbox = InboxStore(client: harness.client, userID: "u")
        defer { inbox.shutdown() }
        await inbox.refresh()
        let refreshing = Task { await inbox.refresh() }
        defer { refreshing.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        await inbox.loadMore()
        refreshing.cancel()
        release.continuation.finish()
        await refreshing.value
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while !inbox.entries.contains(where: { $0.noticeID == "old" }), ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(1))
        }
        #expect(inbox.entries.contains { $0.noticeID == "old" })
        #expect(reads.withLock { $0.older } == 1)
    }

    @Test("旧取得のキャンセル後も待機中の更新が連合通知を取得する")
    func pendingRefreshSurvivesCancellation() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let requests = Mutex(0)
        harness.stub { request in
            if request.url?.path == "/api/shapes/notifications" {
                return (200, ["electric-handle": "notices", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            }
            #expect(request.url?.path == "/api/activitypub/notifications")
            let attempt = requests.withLock { $0 += 1; return $0 }
            if attempt == 1 {
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                throw CancellationError()
            }
            return (200, [:], Data(#"{"items":[],"unread_count":7,"next_cursor":null}"#.utf8))
        }
        let inbox = InboxStore(client: harness.client, userID: "u")
        defer { inbox.shutdown() }
        let first = Task { await inbox.refresh() }
        defer { first.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        let second = Task { await inbox.refresh() }
        defer { second.cancel() }
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while inbox.pendingRefreshCount == 0, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
        try #require(inbox.pendingRefreshCount == 1)
        first.cancel()
        release.continuation.finish()
        await first.value
        await second.value
        #expect(requests.withLock { $0 } == 2)
        #expect(inbox.pendingRefreshCount == 0)
        #expect(inbox.unreadCount == 7 && inbox.error == nil)
    }

    @Test("Watchの自動更新はiPhone側の操作エラーを消さず、手動更新で解除する")
    func automaticRefreshKeepsActionError() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            if request.url?.path == "/api/shapes/notifications" {
                return (200, ["electric-handle": "notices", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            }
            if request.url?.path == "/api/notifications/read-all" { return (500, [:], Data("{}".utf8)) }
            #expect(request.url?.path == "/api/activitypub/notifications")
            return (200, [:], Data(#"{"items":[],"unread_count":1,"next_cursor":null}"#.utf8))
        }
        let inbox = InboxStore(client: harness.client, userID: "u")
        defer { inbox.shutdown() }
        await inbox.refresh()
        await inbox.markAllRead()
        let failure = try #require(inbox.error)
        #expect(failure == "すべての通知を既読にできませんでした。")
        await inbox.refresh(clearActionError: false)
        #expect(inbox.error == failure)
        await inbox.refresh()
        #expect(inbox.error == nil)
    }

    @Test("全既読中の追加申請は再確認を再試行でき、並行した承認を巻き戻さない", arguments: [false, true])
    func markAllRevalidatesConcurrentFollowResponse(failsCheck: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let acknowledged = AsyncStream<Void>.makeStream(), releaseAcknowledgement = AsyncStream<Void>.makeStream()
        let checked = AsyncStream<Void>.makeStream(), releaseCheck = AsyncStream<Void>.makeStream()
        defer {
            acknowledged.continuation.finish(); releaseAcknowledgement.continuation.finish()
            checked.continuation.finish(); releaseCheck.continuation.finish()
        }
        let state = Mutex((approved: false, failHead: false, checks: 0, acknowledgements: 0))
        harness.stub { request in
            func page(old: Bool, pending: Bool, read: Bool) throws -> (Int, [String: String], Data) {
                let row: [String: Any] = ["id": old ? "old" : "new", "actor_id": "https://remote.test/user/member",
                    "display_name": "メンバー", "kind": old ? "follow_request" : "reply", "activity_id": "activity",
                    "created_at": old ? "2026-11-18T09:00:00Z" : "2026-11-18T10:00:00Z", "is_read": read, "pending": pending]
                return (200, [:], try JSONSerialization.data(withJSONObject: ["items": [row], "unread_count": read ? 0 : 2,
                    "next_cursor": old ? NSNull() as Any : "new" as Any]))
            }
            switch request.url?.path {
            case "/api/shapes/notifications":
                return (200, ["electric-handle": "notices", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            case "/api/notifications/read-all":
                state.withLock { $0.acknowledgements += 1 }
                acknowledged.continuation.yield(())
                var iterator = releaseAcknowledgement.stream.makeAsyncIterator()
                _ = await iterator.next()
                return (200, [:], Data("{}".utf8))
            case "/api/activitypub/follows/respond":
                state.withLock { $0.approved = true }
                return (200, [:], Data("{}".utf8))
            case "/api/activitypub/notifications/entries":
                let current = state.withLock { $0.checks += 1; return $0 }
                if current.checks == 1 {
                    checked.continuation.yield(())
                    var iterator = releaseCheck.stream.makeAsyncIterator()
                    _ = await iterator.next()
                    if failsCheck { return (500, [:], Data()) }
                }
                return try page(old: true, pending: !current.approved, read: true)
            default:
                let old = request.url?.query?.contains("before=") == true
                if !old, state.withLock({ $0.failHead }) { throw URLError(.notConnectedToInternet) }
                return try page(old: old, pending: old, read: false)
            }
        }
        let inbox = InboxStore(client: harness.client, userID: "u")
        defer { inbox.shutdown() }
        await inbox.refresh()
        let marking = Task { await inbox.markAllRead() }
        defer { marking.cancel() }
        var ackIterator = acknowledged.stream.makeAsyncIterator()
        try #require(await ackIterator.next() != nil)
        await inbox.loadMore()
        let request = try #require(inbox.entries.first { $0.noticeID == "old" })
        state.withLock { $0.failHead = true }
        releaseAcknowledgement.continuation.finish()
        var checkIterator = checked.stream.makeAsyncIterator()
        try #require(await checkIterator.next() != nil)
        await inbox.respond(request, accept: true)
        releaseCheck.continuation.finish()
        await marking.value
        if failsCheck {
            #expect(inbox.error?.hasPrefix("全既読は完了しましたが") == true)
            await inbox.refresh()
            #expect(inbox.error?.hasPrefix("全既読は完了しましたが") != true)
        }
        let updated = try #require(inbox.entries.first { $0.noticeID == "old" })
        #expect(!updated.pendingFollow && !updated.isUnread)
        #expect(state.withLock { $0.checks } == 2)
        #expect(state.withLock { $0.acknowledgements } == 1)
    }

    @Test("全既読後の履歴照合が失敗しても未読件数を更新し、照合だけを再試行する", arguments: [false, true], [false, true])
    func markAllRefreshesCountAfterReconciliationFailure(visible: Bool, waiting: Bool) async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        let headStarted = AsyncStream<Void>.makeStream(), releaseHead = AsyncStream<Void>.makeStream()
        defer {
            started.continuation.finish(); release.continuation.finish()
            headStarted.continuation.finish(); releaseHead.continuation.finish()
        }
        let state = Mutex((acknowledgements: 0, read: false, failCheck: true, heads: 0, holdHead: waiting))
        harness.stub { request in
            switch request.url?.path {
            case "/api/shapes/notifications":
                return (200, ["electric-handle": "notices", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            case "/api/notifications/read-all":
                state.withLock { $0.acknowledgements += 1 }
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                state.withLock { $0.read = true }
                return (200, [:], Data("{}".utf8))
            default:
                let checking = request.url?.path == "/api/activitypub/notifications/entries"
                let old = checking || request.url?.query?.contains("before=") == true
                let current = state.withLock { value in
                    if !old { value.heads += 1 }
                    let snapshot = value
                    if !old, value.read { value.holdHead = false }
                    return snapshot
                }
                if !old, current.read, current.holdHead {
                    headStarted.continuation.yield(())
                    var iterator = releaseHead.stream.makeAsyncIterator()
                    _ = await iterator.next()
                }
                if checking, current.failCheck { return (500, [:], Data()) }
                let row: [String: Any] = ["id": old ? "old" : "new", "actor_id": "https://remote.test/user/member",
                    "display_name": "メンバー", "kind": "reply", "activity_id": old ? "old" : "new",
                    "created_at": old ? "2026-11-18T09:00:00Z" : "2026-11-18T10:00:00Z", "is_read": current.read, "pending": false]
                return (200, [:], try JSONSerialization.data(withJSONObject: ["items": [row], "unread_count": current.read ? 0 : 3,
                    "next_cursor": old ? NSNull() as Any : "new" as Any]))
            }
        }
        let inbox = InboxStore(client: harness.client, userID: "u")
        defer { inbox.shutdown() }
        await inbox.refresh()
        #expect(inbox.unreadCount == 3)
        let marking = Task { await inbox.markAllRead() }
        defer { marking.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        await inbox.loadMore()
        if visible {
            inbox.setVisible(try #require(inbox.entries.first { $0.noticeID == "old" }), visible: true)
            await inbox.refresh()
        }
        release.continuation.finish()
        if waiting {
            var headIterator = headStarted.stream.makeAsyncIterator()
            try #require(await headIterator.next() != nil)
            state.withLock { $0.failCheck = false }
            let refreshing = Task { await inbox.refresh() }
            defer { refreshing.cancel() }
            let deadline = ContinuousClock.now.advanced(by: .seconds(2))
            while inbox.pendingRefreshCount == 0, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(1)) }
            try #require(inbox.pendingRefreshCount == 1)
            releaseHead.continuation.finish()
            await marking.value
            await refreshing.value
            #expect(inbox.error == nil && inbox.unreadCount == 0)
            #expect(try #require(inbox.entries.first { $0.noticeID == "old" }).isUnread == false)
            #expect(state.withLock { $0.acknowledgements } == 1)
            return
        }
        await marking.value
        #expect(inbox.unreadCount == 0)
        #expect(state.withLock { $0.heads } == (visible ? 3 : 2))
        #expect(inbox.error?.hasPrefix("全既読は完了しましたが") == true)
        #expect(try #require(inbox.entries.first { $0.noticeID == "old" }).isUnread)
        state.withLock { $0.failCheck = false }
        await inbox.refresh()
        #expect(inbox.error == nil && inbox.unreadCount == 0)
        #expect(try #require(inbox.entries.first { $0.noticeID == "old" }).isUnread == false)
        #expect(state.withLock { $0.acknowledgements } == 1)
    }

    @Test("過去行の照合失敗後も先頭行の既読応答で未読件数を二重に減らさない")
    func headReadStateMatchesCountAfterHistoryFailure() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        let read = Mutex(false)
        harness.stub { request in
            switch request.url?.path {
            case "/api/shapes/notifications":
                return (200, ["electric-handle": "notices", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            case "/api/notifications/read":
                read.withLock { $0 = true }
                started.continuation.yield(())
                var iterator = release.stream.makeAsyncIterator()
                _ = await iterator.next()
                return (200, [:], Data("{}".utf8))
            case "/api/activitypub/notifications/entries":
                return (500, [:], Data())
            default:
                let old = request.url?.query?.contains("before=") == true
                let acknowledged = read.withLock { $0 }
                let row: [String: Any] = ["id": old ? "old" : "new", "actor_id": "https://remote.test/user/member",
                    "display_name": "メンバー", "kind": "reply", "activity_id": old ? "old" : "new",
                    "created_at": old ? "2026-11-18T09:00:00Z" : "2026-11-18T10:00:00Z", "is_read": !old && acknowledged, "pending": false]
                return (200, [:], try JSONSerialization.data(withJSONObject: ["items": [row], "unread_count": acknowledged ? 2 : 3,
                    "next_cursor": old ? NSNull() as Any : "new" as Any]))
            }
        }
        let inbox = InboxStore(client: harness.client, userID: "u")
        defer { inbox.shutdown() }
        await inbox.refresh()
        await inbox.loadMore()
        let newest = try #require(inbox.entries.first { $0.noticeID == "new" })
        let marking = Task { await inbox.markRead(newest) }
        defer { marking.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        inbox.setVisible(try #require(inbox.entries.first { $0.noticeID == "old" }), visible: true)
        await inbox.refresh()
        #expect(inbox.error != nil)
        #expect(inbox.unreadCount == 2)
        #expect(try #require(inbox.entries.first { $0.noticeID == "new" }).isUnread == false)
        release.continuation.finish()
        await marking.value
        #expect(inbox.unreadCount == 2)
    }

    @Test("Electric の文字列化された既読値とメッセージの抜粋を読み取る")
    func shape() throws {
        let json = """
        {"id":"n","user_id":"u","actor_id":"a","channel_id":"c","message_id":"m","notification_type":"mention","created_at":"2026-11-18 09:00:00","is_read":"f","display_data":"{\\"message\\":{\\"content\\":\\"こんにちは\\"}}"}
        """
        let row = try JSONDecoder().decode(ChannelNotice.self, from: Data(json.utf8))
        #expect(!row.is_read)
        #expect(row.display?.message?.content == "こんにちは")
    }

    @Test("同じ投稿へのスタンプと連合の重複通知をまとめ、取得済みの時間範囲を表示する")
    func groupedFeed() throws {
        let rows = [("first", "alice", "12:00:02"), ("next", "bob", "12:00:01"), ("old", "alice", "11:59:59")].map { id, actor, time in
            ChannelNotice(id: id, user_id: "u", actor_id: actor, channel_id: "c", message_id: id == "old" ? "other" : "m",
                          notification_type: "reaction", created_at: "2026-11-18T" + time + "Z", is_read: false, display: nil)
        }
        let mirror = FederatedNotice(id: "f", actor_id: "local:alice", display_name: "花", avatar: nil, kind: "like",
                                     activity_id: "activity", post_id: "m", content: nil, created_at: "2026-11-18T12:00:00Z",
                                     is_read: false, pending: false, reaction: "👍")
        let feed = InboxFeed(channels: rows, federated: [mirror], federatedUnread: 2, more: true,
                             acknowledged: [], server: "https://ui.test.invalid")
        #expect(feed.entries.count == 1)
        let group = try #require(feed.entries.first)
        #expect(group.actorIDs == ["alice", "bob"])
        #expect(Set(group.unreadIDs) == ["first", "next", "f"])
        #expect(feed.unreadCount == 4)
    }

    @Test("連合通知の次ページを読み、選択既読と全件既読をサーバーに保存する")
    func pagesAndRead() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let acknowledged = Mutex(Set<String>())
        harness.stub { request in
            let path = request.url?.path
            let parameters = URLComponents(url: try #require(request.url), resolvingAgainstBaseURL: false)?.queryItems
            if path == "/api/shapes/notifications" {
                return (200, ["electric-handle": "notices", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            }
            if path == "/api/notifications/read" {
                let ids = try JSONDecoder().decode([String: [String]].self, from: StubProtocol.body(request))
                #expect(ids["ids"] == ["new"])
                acknowledged.withLock { $0.formUnion(ids["ids"] ?? []) }
                return (200, [:], Data("{}".utf8))
            }
            if path == "/api/notifications/read-all" {
                #expect(request.httpMethod == "POST")
                acknowledged.withLock { $0 = ["new", "old"] }
                return (200, [:], Data("{}".utf8))
            }
            #expect(path == "/api/activitypub/notifications")
            let next = parameters?.first(where: { $0.name == "before" })?.value
            let second = next == "new"
            let read = acknowledged.withLock { $0 }
            let id = second ? "old" : "new"
            let page: [String: Any] = ["items": [["id": id, "actor_id": "https://remote.test/user/person",
                                                  "kind": "reply", "activity_id": "activity", "display_name": "花",
                                                  "is_read": read.contains(id), "pending": false,
                                                  "created_at": second ? "2026-11-18T09:00:00Z" : "2026-11-18T10:00:00Z"]],
                                       "unread_count": 2 - read.count, "next_cursor": second ? NSNull() as Any : "new" as Any]
            return (200, [:], try JSONSerialization.data(withJSONObject: page))
        }
        let inbox = InboxStore(client: harness.client, userID: "u")
        await inbox.refresh()
        #expect(inbox.entries.map(\.noticeID) == ["new"])
        await inbox.loadMore()
        #expect(inbox.entries.map(\.noticeID) == ["new", "old"])
        #expect(!inbox.hasMore)
        let first = try #require(inbox.entries.first)
        let link = try #require(inbox.actorURL(for: first))
        #expect(link.host == "api.test.invalid")
        #expect(link.absoluteString.hasSuffix("home/users/https%3A%2F%2Fremote.test%2Fuser%2Fperson"))
        await inbox.markRead(first)
        #expect(inbox.unreadCount == 1)
        await inbox.markAllRead()
        #expect(inbox.unreadCount == 0)
        #expect(inbox.entries.allSatisfy { !$0.isUnread })
    }

    @Test("先頭更新は過去の通知を保持し、表示中の行を再認可して新着の空白だけを追加で読む")
    func retainsPagesAcrossRefresh() async throws {
        struct Requests {
            var stage = 0
            var cursors: [String] = []
            var checked: [[String]] = []
        }
        let requests = Mutex(Requests())
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            let url = try #require(request.url)
            if url.path == "/api/shapes/notifications" {
                return (200, ["electric-handle": "notices", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            }
            let stage = requests.withLock { $0.stage }
            let ids: [Int]
            let next: String?
            if url.path == "/api/activitypub/notifications/entries" {
                let body = try JSONDecoder().decode([String: [String]].self, from: StubProtocol.body(request))
                let checked = try #require(body["entries"])
                requests.withLock { $0.checked.append(checked) }
                ids = checked.compactMap(Int.init).filter { $0 != 3 }
                next = nil
            } else {
                let cursor = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first?.value ?? ""
                requests.withLock { $0.cursors.append(cursor) }
                switch (stage, cursor) {
                case (0, ""): (ids, next) = ([5, 4], "4")
                case (0, "4"): (ids, next) = ([3, 2], "2")
                case (1, ""): (ids, next) = ([6, 5], "5")
                case (2, ""): (ids, next) = ([11, 10], "10")
                case (2, "10"): (ids, next) = ([9, 8], "8")
                case (2, "8"): (ids, next) = ([7, 6], "6")
                case (2, "2"): (ids, next) = ([1], nil)
                default:
                    Issue.record("予期しないページの要求")
                    (ids, next) = ([], nil)
                }
            }
            let items: [[String: Any]] = ids.map { id in
                ["id": String(id), "actor_id": "actor", "kind": "like", "activity_id": "activity",
                 "display_name": "花", "is_read": stage > 0 && id == 4, "pending": false,
                 "created_at": Date(timeIntervalSince1970: Double(id)).ISO8601Format()]
            }
            let page: [String: Any] = ["items": items, "unread_count": 5, "next_cursor": next as Any? ?? NSNull()]
            return (200, [:], try JSONSerialization.data(withJSONObject: page))
        }
        let store = InboxStore(client: harness.client, userID: "u")
        defer { store.shutdown() }
        await store.refresh()
        await store.loadMore()
        #expect(store.entries.map(\.noticeID) == ["5", "4", "3", "2"])
        for entry in store.entries where ["3", "4"].contains(entry.noticeID) { store.setVisible(entry, visible: true) }
        requests.withLock { $0.stage = 1 }
        await store.refresh()
        #expect(store.entries.map(\.noticeID) == ["6", "5", "4", "2"])
        #expect(store.entries.first(where: { $0.noticeID == "4" })?.isUnread == false)
        #expect(requests.withLock { $0.cursors } == ["", "4", ""])
        #expect(requests.withLock { $0.checked } == [["3", "4"]])

        requests.withLock { $0.stage = 2 }
        await store.refresh()
        #expect(store.entries.map(\.noticeID) == ["11", "10", "6", "5", "4", "2"])
        await store.loadMore()
        await store.loadMore()
        await store.loadMore()
        #expect(!store.hasMore)
        #expect(store.entries.map(\.noticeID) == ["11", "10", "9", "8", "7", "6", "5", "4", "2", "1"])
        #expect(requests.withLock { Array($0.cursors.suffix(4)) } == ["", "10", "8", "2"])
    }

    enum DeletedBoundary: Sendable { case head, visibleOlder, hiddenOlder }

    @Test("ページ末尾の通知が削除されても残りの通知を読み込める",
          arguments: [DeletedBoundary.head, .visibleOlder, .hiddenOlder])
    func resumesAfterDeletedBoundary(_ scenario: DeletedBoundary) async throws {
        struct State {
            var ids = [6, 5, 4, 3, 2, 1]
            var before: [String] = []
        }
        let state = Mutex(State())
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            let url = try #require(request.url)
            if url.path == "/api/shapes/notifications" {
                return (200, ["electric-handle": "notices", "electric-offset": "1_0", "electric-up-to-date": "true"], Data("[]".utf8))
            }
            let all = state.withLock { $0.ids }
            let results: [Int]
            if url.path.hasSuffix("/entries") {
                let body = try JSONDecoder().decode([String: [String]].self, from: StubProtocol.body(request))
                results = all.filter { body["entries"]?.contains(String($0)) == true }
            } else {
                let cursor = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first?.value
                state.withLock { $0.before.append(cursor ?? "") }
                if let cursor {
                    results = all.contains(where: { String($0) == cursor }) ? all.filter { $0 < (Int(cursor) ?? 0) } : []
                } else { results = all }
            }
            let limit = url.path.hasSuffix("/entries") ? 100 : 2
            let rows: [[String: Any]] = results.prefix(limit).map { id in
                ["id": String(id), "actor_id": "actor", "display_name": "花", "kind": "like", "activity_id": "activity",
                 "created_at": Date(timeIntervalSince1970: Double(id)).ISO8601Format(), "is_read": true, "pending": false]
            }
            let cursor: Any = results.count > limit ? String(results[limit - 1]) as Any : NSNull()
            let page: [String: Any] = ["items": rows, "unread_count": 0, "next_cursor": cursor]
            return (200, [:], try JSONSerialization.data(withJSONObject: page))
        }

        let store = InboxStore(client: harness.client, userID: "u")
        defer { store.shutdown() }
        await store.refresh()
        if scenario != .head { await store.loadMore() }
        if scenario == .visibleOlder {
            store.setVisible(try #require(store.entries.first { $0.noticeID == "3" }), visible: true)
        }
        let deleted = scenario == .head ? 5 : 3
        state.withLock { $0.ids.removeAll { $0 == deleted }; $0.before = [] }
        await store.refresh()
        await store.loadMore()
        if scenario == .head {
            #expect(store.entries.map(\.noticeID) == ["6", "4", "3", "2"])
            #expect(store.hasMore)
        } else {
            #expect(store.entries.map(\.noticeID) == ["6", "5", "4", "2", "1"])
            #expect(!store.hasMore)
        }
        let expected = scenario == .hiddenOlder ? ["", "3", "4"] : ["", "4"]
        #expect(state.withLock { $0.before } == expected)
    }
}
