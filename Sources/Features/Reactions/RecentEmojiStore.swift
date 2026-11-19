import Foundation
import Observation

@Observable @MainActor
final class RecentEmojiStore {
    private(set) var values: [String]
    private let defaults: UserDefaults?
    private let key: String
    static let limit = 16

    init(server: String, userID: String, defaults: UserDefaults? = .standard) {
        self.defaults = defaults
        let scope = server.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + "\n" + userID.lowercased()
        key = "recent-emojis." + Data(scope.utf8).base64EncodedString()
        values = Self.unique(defaults?.stringArray(forKey: key) ?? [])
    }

    func record(_ emoji: String) {
        values = Self.unique([emoji] + values)
        defaults?.set(values, forKey: key)
    }

    func choices(fallback: [String], canonical: (String) -> String) -> [String] {
        Array(Self.unique((values + fallback).map(canonical)).prefix(6))
    }

    private static func unique(_ values: [String]) -> [String] {
        var seen: Set<String> = []
        return Array(values.filter { !$0.isEmpty && $0.utf8.count <= 256 && !$0.hasPrefix("apub:") && seen.insert($0).inserted }
            .prefix(limit))
    }
}
