import Foundation
import Observation

@MainActor
protocol RecentEmojiRemote {
    func recentEmojiHistory() async throws -> [String]
    func recordRecentEmoji(_ emoji: String) async throws -> [String]
}

@Observable @MainActor
final class RecentEmojiStore {
    private(set) var values: [String]
    private let defaults: UserDefaults?
    private let key: String
    private let remote: (any RecentEmojiRemote)?
    @ObservationIgnored private var active = true
    @ObservationIgnored private var pending: Task<Void, Never>?
    static let limit = 16

    init(server: String, userID: String, defaults: UserDefaults? = .standard, remote: (any RecentEmojiRemote)? = nil) {
        self.remote = remote
        self.defaults = defaults
        let scope = server.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + "\n" + userID.lowercased()
        key = "recent-emojis." + Data(scope.utf8).base64EncodedString()
        values = Self.unique(defaults?.stringArray(forKey: key) ?? [])
    }

    func record(_ emoji: String) {
        values = Self.unique([emoji] + values)
        defaults?.set(values, forKey: key)
        sync(emoji)
    }

    func stop() { active = false; pending?.cancel(); pending = nil }

    func refresh() { sync(nil) }

    private func sync(_ emoji: String?) {
        guard active, let remote else { return }
        let previous = pending
        pending = Task { [weak self] in
            await previous?.value
            guard let self, active else { return }
            do {
                let result: [String]
                if let emoji { result = try await remote.recordRecentEmoji(emoji) }
                else { result = try await remote.recentEmojiHistory() }
                guard active else { return }
                values = Self.unique(result)
                defaults?.set(values, forKey: key)
            } catch { /* Keep this account's cached choices while offline. */ }
        }
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
