import Foundation
import Observation

@Observable @MainActor
final class ProfileViewModel {
    private let repository: any WorkspaceRepository
    init(repository: any WorkspaceRepository) { self.repository = repository }
    var me: Person? { repository.state.me }
    func updateName(_ name: String) async throws {
        let value = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty, value.unicodeScalars.count <= 80 else {
            throw ZooError.auth("表示名は1〜80文字で入力してください。")
        }
        try await repository.updateName(value)
    }
}
