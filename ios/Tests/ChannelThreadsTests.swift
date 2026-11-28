import Foundation
import Testing
@testable import Zoo

@MainActor
@Suite("参加中のスレッド一覧")
struct ChannelThreadsTests {
    @Test("会話の失効時にスレッド本文を消し、待機中の一覧で復活させない")
    func revocationDropsLateThreads() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let threads = ChannelThreads(client: harness.client)
        defer { threads.shutdown() }
        let page = Data(#"{"items":[{"id":"a","channel_id":"revoked","content":"失効対象の本文","reply_count":1},{"id":"b","channel_id":"other","content":"別の会話","reply_count":1}],"next_cursor":null}"#.utf8)
        harness.stub { _ in (200, [:], page) }
        await threads.refresh()
        try #require(threads.byChannel["revoked"]?.count == 1)
        let started = AsyncStream<Void>.makeStream(), release = AsyncStream<Void>.makeStream()
        defer { started.continuation.finish(); release.continuation.finish() }
        harness.stub { _ in
            started.continuation.yield(())
            for await _ in release.stream { break }
            return (200, [:], page)
        }
        let pending = Task {
            defer { started.continuation.finish() }
            await threads.refresh()
        }
        defer { pending.cancel() }
        var iterator = started.stream.makeAsyncIterator()
        try #require(await iterator.next() != nil)
        threads.revokeChannel("revoked")
        #expect(threads.byChannel["revoked"] == nil && !threads.loading)
        release.continuation.finish()
        await pending.value
        #expect(threads.byChannel["revoked"] == nil)
        #expect(threads.byChannel["other"]?.map(\.id) == ["b"])
    }

    @Test("カーソルで次ページを追加し、更新時は読み込んだ範囲から消えたスレッドを除く")
    func expandedRange() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let threads = ChannelThreads(client: harness.client)
        func serve(_ page: String, query: [String: String]) {
            harness.stub { request in
                #expect(request.url?.path == "/api/threads")
                let params = URLComponents(url: try #require(request.url), resolvingAgainstBaseURL: false)?.queryItems ?? []
                try #require(Set(params.map(\.name)).count == params.count, "クエリ項目名が重複しています。")
                #expect(Dictionary(uniqueKeysWithValues: params.map { ($0.name, $0.value ?? "") }) == query)
                return (200, [:], Data(page.utf8))
            }
        }
        serve(#"{"items":[{"id":"a","channel_id":"c","content":"先頭","reply_count":1}],"next_cursor":"1 / 前"}"#, query: [:])
        await threads.refresh()
        #expect(threads.byChannel["c"]?.map(\.id) == ["a"])
        serve(#"{"items":[{"id":"b","channel_id":"c","content":"次","reply_count":2}],"next_cursor":"2 / 前"}"#, query: ["before": "1 / 前"])
        await threads.loadMore()
        #expect(threads.byChannel["c"]?.map(\.id) == ["a", "b"])
        serve(#"{"items":[{"id":"b","channel_id":"c","content":"更新","reply_count":3}],"next_cursor":null}"#, query: ["through": "2 / 前"])
        await threads.refresh()
        #expect(threads.byChannel["c"]?.map(\.id) == ["b"])
        #expect(threads.byChannel["c"]?.first?.reply_count == 3)
        #expect(threads.hasMore)
        #expect(threads.error == nil)
    }
}
