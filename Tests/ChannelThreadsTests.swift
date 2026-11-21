import Foundation
import Testing
@testable import Zoo

@MainActor
@Suite("参加中のスレッド一覧")
struct ChannelThreadsTests {
    @Test("カーソルで次ページを追加し、更新時は読み込んだ範囲から消えたスレッドを除く")
    func expandedRange() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        let threads = ChannelThreads(client: harness.client)
        func serve(_ page: String, query: [String: String]) {
            harness.stub { request in
                #expect(request.url?.path == "/api/threads")
                let params = URLComponents(url: try #require(request.url), resolvingAgainstBaseURL: false)?.queryItems ?? []
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
