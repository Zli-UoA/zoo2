import Foundation
import Testing
@testable import Zoo

@MainActor
struct PreviewE2eeTests {
    @Test func 初期設定に失敗したら対象の端末を始めずエラーを残す() async throws {
        let controller = E2eeController.makePreview(userID: UUID().uuidString, alreadySetUp: true, failingSetup: true)
        for _ in 0..<100 where controller.statusError == nil { try await Task.sleep(for: .milliseconds(100)) }
        #expect(controller.statusError?.contains("初期設定に失敗") == true)
        // 始めていないので、状態の取得でエラーが消えることも、解錠済みになることもない。
        try await Task.sleep(for: .milliseconds(500))
        #expect(controller.statusError?.contains("初期設定に失敗") == true)
        #expect(controller.status == .starting)
    }
}
