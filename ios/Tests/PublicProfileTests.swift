import Foundation
import Testing
@testable import Zoo

@MainActor
@Suite("公開プロフィール")
struct PublicProfileTests {
    @Test("公開名の取得と更新はサーバーが保存した値を返す")
    func loadAndSave() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { request in
            #expect(request.url?.path == "/api/activitypub/profile")
            if request.httpMethod == "GET" {
                return (200, [:], Data(#"{"display_name":"","default_display_name":"動物園","summary":""}"#.utf8))
            }
            #expect(request.httpMethod == "PUT")
            let fields = try JSONDecoder().decode([String: String].self, from: StubProtocol.body(request))
            #expect(fields == ["display_name": " 花 ", "summary": "こんにちは🌸"])
            return (200, [:], Data(#"{"display_name":"花","default_display_name":"動物園","summary":"応答側で整えた自己紹介🌸"}"#.utf8))
        }
        #expect(try await harness.client.publicProfile().name == "動物園")
        let updated = try await harness.client.updatePublicProfile(name: " 花 ", summary: "こんにちは🌸")
        #expect(updated.name == "花")
        #expect(updated.summary == "応答側で整えた自己紹介🌸")
        #expect(harness.client.accountProfileURL?.absoluteString == "https://id.test.invalid/settings/profile")
    }

    @Test("表示名と自己紹介の上限をサーバーと同じUnicodeスカラー数で判定する")
    func profileLimits() {
        var draft = PublicProfile(display_name: String(repeating: "👩‍💻", count: 33) + "a", default_display_name: "自分",
                                  summary: String(repeating: "👩‍💻", count: 333) + "a")
        #expect(draft.canSave)
        draft.display_name += "b"
        #expect(!draft.canSave)
        draft.display_name = "👩‍💻"
        draft.summary += "b"
        #expect(!draft.canSave)
    }
}
